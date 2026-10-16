//! Read-only counterparts of Firestorm getZOffsets/getScriptInfo
//! (fsradar.cpp and fs_resources/EBEDD1D2-*.lsltxt, originally LGPL 2.1).
//! They never consume the movement sequence or refresh its watchdog.
use super::{Endpoint, http, transport_failure, valid_url};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

pub const MAX_AVATARS: usize = 32;
const MAX_REPLY: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Altitudes(Vec<Uuid>),
    ScriptInfo(Uuid),
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

#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Altitudes(Vec<Altitude>),
    ScriptInfo(ScriptInfo),
}

impl Query {
    pub fn valid(&self) -> bool {
        match self {
            Self::Altitudes(ids) => {
                !ids.is_empty()
                    && ids.len() <= MAX_AVATARS
                    && ids.iter().all(|id| !id.is_nil())
                    && ids.iter().collect::<HashSet<_>>().len() == ids.len()
            }
            Self::ScriptInfo(id) => !id.is_nil(),
        }
    }
    fn body(&self, token: Uuid) -> String {
        match self {
            Self::Altitudes(ids) => format!(
                "query|{token}|altitudes|{}",
                ids.iter().map(Uuid::to_string).collect::<Vec<_>>().join(",")
            ),
            Self::ScriptInfo(id) => format!("query|{token}|scripts|{id}"),
        }
    }
    pub fn decode(&self, bytes: &[u8]) -> Option<Reply> {
        if !self.valid() || bytes.len() > MAX_REPLY {
            return None;
        }
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let values = value.as_array()?;
        match self {
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
            Self::ScriptInfo(object) => {
                if values.len() != 7 || Uuid::parse_str(values[0].as_str()?).ok()? != *object {
                    return None;
                }
                let name = String::from_utf8(STANDARD.decode(values[1].as_str()?).ok()?).ok()?;
                if name.len() > 256 {
                    return None;
                }
                let integer = |i: usize| u32::try_from(values[i].as_u64()?).ok().filter(|v| *v <= i32::MAX as u32);
                let time = |i: usize| values[i].as_f64().filter(|v| v.is_finite() && (0.0..=3600.0).contains(v));
                let running = integer(2)?;
                let total = integer(3)?;
                if running > total {
                    return None;
                }
                Some(Reply::ScriptInfo(ScriptInfo {
                    object: *object,
                    name: name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect(),
                    running,
                    total,
                    memory: integer(4)?,
                    cpu_seconds: time(5)?,
                    character_seconds: time(6)?,
                }))
            }
        }
    }
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
        return Err(if response.status().as_u16() == 404 {
            aurora_i18n::tr!("bridge-script-info-missing").into()
        } else {
            aurora_i18n::tr!("bridge-request-http", status = response.status().as_u16().to_string())
        });
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
        for ids in [vec![], vec![Uuid::nil()], vec![id(1), id(1)], (1..=33).map(id).collect()] {
            assert!(!Query::Altitudes(ids).valid());
        }
        let query = Query::Altitudes((1..=32).map(id).collect());
        assert!(query.valid());
        let body = query.body(id(100));
        assert!(body.starts_with("query|") && body.len() <= 1536);
        assert!(!body.contains("lock|") && !body.contains("https://"));
        assert!(!Query::ScriptInfo(Uuid::nil()).valid());
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
    #[tokio::test]
    async fn invalid_private_endpoint_is_rejected_without_network() {
        let endpoint = Endpoint {
            url: "https://localhost/synthetic".into(),
            token: id(10),
        };
        assert!(send(endpoint, Query::ScriptInfo(id(1))).await.is_err());
    }
}
