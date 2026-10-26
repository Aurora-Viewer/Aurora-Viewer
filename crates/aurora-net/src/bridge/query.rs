//! Query services and explicit photo-posing consent relay. Read-only services
//! correspond to getZOffsets/getScriptInfo in the Legacy Viewer
//! (fsradar.cpp and fs_resources/EBEDD1D2-*.lsltxt, originally LGPL 2.1).
//! They never consume the movement sequence or refresh its watchdog.
use super::{Endpoint, gone_failure, http, transport_failure, valid_url};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

pub const MAX_AVATARS: usize = 16;
const MAX_REPLY: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Portal { target: Uuid, message: super::portal::Message },
    Pose { request: Uuid, kind: PoseKind, target: Uuid },
    Altitudes(Vec<Uuid>),
    ScriptInfo(Uuid),
    ObjectInfo(Uuid),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoseKind {
    Request,
    Accept,
    Refuse,
    Cancel,
    Withdraw,
}
impl PoseKind {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Accept => "accept",
            Self::Refuse => "refuse",
            Self::Cancel => "cancel",
            Self::Withdraw => "withdraw",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "request" => Self::Request,
            "accept" => Self::Accept,
            "refuse" => Self::Refuse,
            "cancel" => Self::Cancel,
            "withdraw" => Self::Withdraw,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Altitude {
    pub avatar: Uuid,
    pub z: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScriptInfo {
    pub object: Uuid,
    pub name: String,
    pub running: u32,
    pub total: u32,
    /// Simulator-reported allowed memory, not live heap usage.
    pub memory: u32,
    pub cpu_seconds: f64,
    pub character_seconds: f64,
}

/// Extended llGetObjectDetails report, including the basic script statistics.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectInfo {
    pub scripts: ScriptInfo,
    pub description: String,
    pub root: Uuid,
    pub prims: u32,
    pub land_impact: u32,
    pub inventory: u32,
    pub velocity: [f64; 3],
    pub position: [f64; 3],
    pub rotation: [f64; 4],
    pub rotation_degrees: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub creator: Uuid,
    pub owner: Uuid,
    pub previous_owner: Uuid,
    pub rezzer: Uuid,
    pub group: Uuid,
    pub created: String,
    pub rezzed: String,
    pub pathfinding: i8,
    pub attachment_point: u8,
    pub temporary_attachment: bool,
    pub observer_position: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Portal,
    Pose,
    Altitudes(Vec<Altitude>),
    ScriptInfo(ScriptInfo),
    ObjectInfo(Box<ObjectInfo>),
}

impl Query {
    pub fn valid(&self) -> bool {
        match self {
            Self::Portal { target, message } => {
                message.encode().is_some()
                    && (!matches!(
                        message,
                        super::portal::Message::Request { .. } | super::portal::Message::Details { .. }
                    ) || !target.is_nil())
            }
            Self::Pose { request, target, .. } => !request.is_nil() && !target.is_nil(),
            Self::Altitudes(ids) => {
                !ids.is_empty()
                    && ids.len() <= MAX_AVATARS
                    && ids.iter().all(|id| !id.is_nil())
                    && ids.iter().collect::<HashSet<_>>().len() == ids.len()
            }
            Self::ScriptInfo(id) | Self::ObjectInfo(id) => !id.is_nil(),
        }
    }
    fn body(&self, token: Uuid) -> String {
        match self {
            Self::Portal { target, message } => format!("query|{token}|portal|{target},{}", message.encode().unwrap_or_default()),
            Self::Pose { request, kind, target } => format!("query|{token}|pose|{},{request},{target}", kind.wire()),
            Self::Altitudes(ids) => format!(
                "query|{token}|altitudes|{}",
                ids.iter().map(Uuid::to_string).collect::<Vec<_>>().join(",")
            ),
            Self::ScriptInfo(id) => format!("query|{token}|scripts|{id}"),
            Self::ObjectInfo(id) => format!("query|{token}|object|{id}"),
        }
    }
    pub fn decode(&self, bytes: &[u8]) -> Option<Reply> {
        if !self.valid() || bytes.len() > MAX_REPLY {
            return None;
        }
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let values = value.as_array()?;
        match self {
            Self::Portal { .. } => values.is_empty().then_some(Reply::Portal),
            Self::Pose { .. } => values.is_empty().then_some(Reply::Pose),
            Self::Altitudes(requested) => {
                if values.len() > requested.len() * 2 || values.len() % 2 != 0 {
                    return None;
                }
                let mut seen = HashSet::new();
                let mut out = Vec::new();
                for pair in values.as_chunks::<2>().0 {
                    let avatar = Uuid::parse_str(pair[0].as_str()?).ok()?;
                    let z = pair[1].as_f64()?;
                    if !requested.contains(&avatar) || !seen.insert(avatar) || !z.is_finite() || z.abs() > 1_000_000.0 {
                        return None;
                    }
                    out.push(Altitude { avatar, z: z as f32 });
                }
                Some(Reply::Altitudes(out))
            }
            Self::ScriptInfo(object) | Self::ObjectInfo(object) => {
                let extended = matches!(self, Self::ObjectInfo(_));
                let expected = if extended { 28 } else { 7 };
                if values.len() != expected || Uuid::parse_str(values[0].as_str()?).ok()? != *object {
                    return None;
                }
                let name = text(&values[1], 256)?;
                let integer = |i: usize| u32::try_from(values[i].as_u64()?).ok().filter(|v| *v <= i32::MAX as u32);
                let time = |i: usize| values[i].as_f64().filter(|v| v.is_finite() && (0.0..=3600.0).contains(v));
                let running = integer(2)?;
                let total = integer(3)?;
                if running > total {
                    return None;
                }
                let scripts = ScriptInfo {
                    object: *object,
                    name,
                    running,
                    total,
                    memory: integer(4)?,
                    cpu_seconds: time(5)?,
                    character_seconds: time(6)?,
                };
                if !extended {
                    return Some(Reply::ScriptInfo(scripts));
                }
                let id = |i: usize| Uuid::parse_str(values[i].as_str()?).ok();
                let timestamp = |i: usize| {
                    let value = values[i].as_str()?;
                    (value.len() <= 64 && value.chars().all(|c| c.is_ascii_digit() || "-:.TZ+".contains(c))).then(|| value.to_owned())
                };
                Some(Reply::ObjectInfo(Box::new(ObjectInfo {
                    scripts,
                    description: text(&values[7], 256)?,
                    root: id(8)?,
                    prims: integer(9)?,
                    land_impact: integer(10)?,
                    inventory: integer(11)?,
                    velocity: components(&values[12])?,
                    position: components(&values[13])?,
                    rotation: components(&values[14])?,
                    angular_velocity: components(&values[15])?,
                    creator: id(16)?,
                    owner: id(17)?,
                    previous_owner: id(18)?,
                    rezzer: id(19)?,
                    group: id(20)?,
                    created: timestamp(21)?,
                    rezzed: timestamp(22)?,
                    pathfinding: i8::try_from(values[23].as_i64()?).ok().filter(|v| (-1..=6).contains(v))?,
                    attachment_point: u8::try_from(integer(24)?).ok()?,
                    temporary_attachment: match integer(25)? {
                        0 => false,
                        1 => true,
                        _ => return None,
                    },
                    observer_position: components(&values[26])?,
                    rotation_degrees: components(&values[27])?,
                })))
            }
        }
    }
}

fn text(value: &Value, limit: usize) -> Option<String> {
    let decoded = String::from_utf8(STANDARD.decode(value.as_str()?).ok()?).ok()?;
    (decoded.len() <= limit).then(|| decoded.chars().map(|c| if c.is_control() { ' ' } else { c }).collect())
}

fn components<const N: usize>(value: &Value) -> Option<[f64; N]> {
    let value = value.as_str()?.strip_prefix('<')?.strip_suffix('>')?;
    let mut parts = value.split(',');
    let mut result = [0.0_f64; N];
    for component in &mut result {
        *component = parts.next()?.trim().parse().ok()?;
        if !component.is_finite() || component.abs() > 1_000_000.0 {
            return None;
        }
    }
    parts.next().is_none().then_some(result)
}

pub async fn send(endpoint: Endpoint, query: Query) -> Result<Reply, String> {
    if !valid_url(&endpoint.url) || endpoint.token.is_nil() || !query.valid() {
        return Err(aurora_i18n::tr!("bridge-request-failed").into());
    }
    let mut response = http()?
        .post(&endpoint.url)
        .timeout(Duration::from_secs(3))
        .header("Content-Type", "text/plain; charset=utf-8")
        .body(query.body(endpoint.token))
        .send()
        .await
        .map_err(transport_failure)?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        if status == 404 {
            // query.lsl's own 404 names a missing object; a simulator 404
            // means its URL died. Only the first short chunk is read.
            let body = response.chunk().await.ok().flatten();
            return Err(if body.is_some_and(|body| &body[..] == b"unavailable") {
                aurora_i18n::tr!("bridge-script-info-missing").into()
            } else {
                gone_failure()
            });
        }
        return Err(aurora_i18n::tr!("bridge-request-http", status = status.to_string()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_failure)? {
        if bytes.len() + chunk.len() > MAX_REPLY {
            return Err(aurora_i18n::tr!("bridge-request-response").into());
        }
        bytes.extend_from_slice(&chunk);
    }
    query
        .decode(&bytes)
        .ok_or_else(|| aurora_i18n::tr!("bridge-request-response").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    #[test]
    fn bounded_queries_have_no_movement_fields_and_reject_empty_duplicate_targets() {
        for ids in [vec![], vec![Uuid::nil()], vec![id(1), id(1)], (1..=17).map(id).collect()] {
            assert!(!Query::Altitudes(ids).valid());
        }
        let query = Query::Altitudes((1..=16).map(id).collect());
        assert!(query.valid());
        let body = query.body(id(100));
        // bridge.lsl denies longer queries and reads the keys at a fixed
        // 37-character stride, without splitting the body into lists.
        assert!(body.starts_with("query|") && body.len() <= 768);
        assert_eq!(&body[42..53], "|altitudes|");
        assert_eq!(body.len() - 53, 16 * 37 - 1);
        assert!(!body.contains("lock|") && !body.contains("https://"));
        assert!(!Query::ScriptInfo(Uuid::nil()).valid());
        assert!(!Query::ObjectInfo(Uuid::nil()).valid());
        assert_eq!(
            Query::ObjectInfo(id(1)).body(id(100)),
            format!("query|{}|object|{}", id(100), id(1))
        );
    }
    #[test]
    fn altitude_replies_require_requested_unique_finite_samples_and_allow_partial_absence() {
        let query = Query::Altitudes(vec![id(1), id(2)]);
        let decode = |value: Value| query.decode(value.to_string().as_bytes());
        assert_eq!(
            decode(json!([id(1).to_string(), 2048.25])),
            Some(Reply::Altitudes(vec![Altitude { avatar: id(1), z: 2048.25 }]))
        );
        assert_eq!(decode(json!([])), Some(Reply::Altitudes(vec![])));
        for value in [
            json!([id(3).to_string(), 2000]),
            json!([id(1).to_string(), 2, id(1).to_string(), 3]),
            json!([id(1).to_string()]),
            json!([id(1).to_string(), null]),
            json!([id(1).to_string(), "NaN"]),
            json!([id(1).to_string(), 1e100]),
            json!({"altitude": 2000}),
        ] {
            assert!(decode(value).is_none());
        }
        assert!(query.decode(&vec![b' '; MAX_REPLY + 1]).is_none());
    }
    #[test]
    fn script_info_handles_json_shaped_names_without_changing_metric_types_or_units() {
        let query = Query::ScriptInfo(id(1));
        let value = json!([id(1).to_string(), STANDARD.encode("[3], |\nÉtoile"), 2, 3, 196608, 0.00125, 0.0002]);
        let Some(Reply::ScriptInfo(info)) = query.decode(value.to_string().as_bytes()) else {
            panic!("script info")
        };
        assert_eq!((info.running, info.total, info.memory), (2, 3, 196608));
        assert_eq!(info.name, "[3], | Étoile");
        assert_eq!(info.cpu_seconds, 0.00125);
        for (index, bad) in [
            (0, json!(id(2).to_string())),
            (1, json!("!bad base64!")),
            (2, json!(4)),
            (3, json!(-1)),
            (4, json!(-1)),
            (5, json!(-0.1)),
            (6, json!(1e100)),
        ] {
            let mut bad_value = value.clone();
            bad_value[index] = bad;
            assert!(query.decode(bad_value.to_string().as_bytes()).is_none(), "field {index}");
        }
    }
    fn object_reply() -> Value {
        json!([
            id(1).to_string(),
            STANDARD.encode("[3], |\nÉtoile"),
            2,
            3,
            196608,
            0.00125,
            0.0002,
            STANDARD.encode("Description, |\n[true]"),
            id(2).to_string(),
            3,
            2,
            5,
            "<1.5, -2.0, 0.0>",
            "<128.0, 130.0, 26.0>",
            "<0.0, 0.0, 0.6, 0.8>",
            "<0.0, 0.0, 1.0>",
            id(3).to_string(),
            Uuid::nil().to_string(),
            id(4).to_string(),
            id(5).to_string(),
            id(6).to_string(),
            "2026-10-17T10:00:00.123456Z",
            "2026-10-17T11:00:00Z",
            -1,
            31,
            1,
            "<128.0, 126.0, 26.0>",
            "<0.0, 0.0, 73.739795>"
        ])
    }
    #[test]
    fn object_info_preserves_extended_metrics_null_owners_and_attachment_pathfinding() {
        let value = object_reply();
        let Some(Reply::ObjectInfo(info)) = Query::ObjectInfo(id(1)).decode(value.to_string().as_bytes()) else {
            panic!("extended object info")
        };
        assert_eq!(info.scripts.name, "[3], | Étoile");
        assert_eq!(info.description, "Description, | [true]");
        assert_eq!(info.root, id(2));
        assert_eq!((info.prims, info.land_impact, info.inventory), (3, 2, 5));
        assert_eq!(info.velocity, [1.5, -2.0, 0.0]);
        assert_eq!(info.position, [128.0, 130.0, 26.0]);
        assert_eq!(info.rotation, [0.0, 0.0, 0.6, 0.8]);
        assert_eq!(info.rotation_degrees, [0.0, 0.0, 73.739795]);
        assert_eq!(info.angular_velocity, [0.0, 0.0, 1.0]);
        assert_eq!(
            (info.creator, info.owner, info.previous_owner, info.rezzer, info.group),
            (id(3), Uuid::nil(), id(4), id(5), id(6))
        );
        assert_eq!(info.created, "2026-10-17T10:00:00.123456Z");
        assert_eq!(info.rezzed, "2026-10-17T11:00:00Z");
        assert_eq!((info.pathfinding, info.attachment_point, info.temporary_attachment), (-1, 31, true));
        assert_eq!(info.observer_position, [128.0, 126.0, 26.0]);
        assert!(Query::ScriptInfo(id(1)).decode(value.to_string().as_bytes()).is_none());
        let mut absent_dates = value;
        absent_dates[21] = json!("");
        absent_dates[22] = json!("");
        assert!(Query::ObjectInfo(id(1)).decode(absent_dates.to_string().as_bytes()).is_some());
    }
    #[test]
    fn object_info_rejects_mismatched_truncated_and_malformed_external_fields() {
        let query = Query::ObjectInfo(id(1));
        for (index, bad) in [
            (0, json!(id(2).to_string())),
            (7, json!("bad base64")),
            (7, json!(STANDARD.encode("x".repeat(257)))),
            (8, json!("not a key")),
            (9, json!(-1)),
            (10, json!(1.5)),
            (11, json!(2147483648_u64)),
            (12, json!("<NaN, 0, 0>")),
            (13, json!("<1, 2>")),
            (14, json!("<0, 0, 0, 1, 2>")),
            (15, json!("<0, inf, 0>")),
            (16, json!("bad key")),
            (21, json!("2026\nHTTP body")),
            (23, json!(-2)),
            (24, json!(256)),
            (25, json!(2)),
            (26, json!("<0, 0, 1e100>")),
            (27, json!("0, 0, 90")),
        ] {
            let mut value = object_reply();
            value[index] = bad;
            assert!(query.decode(value.to_string().as_bytes()).is_none(), "field {index}");
        }
        let value = object_reply();
        for count in 0..28 {
            let bytes = serde_json::to_vec(&value.as_array().expect("array")[..count]).expect("synthetic JSON");
            assert!(query.decode(&bytes).is_none(), "truncated at {count}");
        }
    }
    #[tokio::test]
    async fn invalid_private_endpoint_is_rejected_without_network() {
        let endpoint = Endpoint {
            url: "https://localhost/synthetic".into(),
            token: id(10),
        };
        assert!(send(endpoint, Query::ScriptInfo(id(1))).await.is_err());
    }
}
