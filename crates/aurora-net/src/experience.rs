//! Experiences: their details and the agent's permission for each of them,
//! like LLExperienceCache (indra/llmessage/llexperiencecache.cpp, Firestorm,
//! originally LGPL 2.1). `GetExperienceInfo` gives the details of a batch
//! of experiences (requestExperiences, requestExperiencesCoro);
//! `ExperiencePreferences` reads, sets (Allow / Block) and forgets the
//! agent's permission (getExperiencePermission, setExperiencePermission,
//! forgetExperiencePermission).

use crate::caps::CapsError;
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

/// LLExperienceCache::PROPERTY_INVALID: no such experience, or its details
/// could not be read.
pub const PROPERTY_INVALID: i32 = 1 << 0;
/// LLExperienceCache::PROPERTY_GRID: a grid-wide experience (else land-scope).
pub const PROPERTY_GRID: i32 = 1 << 4;

/// requestExperiences: a request is sent once its URL is longer than this.
const URL_SEND_THRESHOLD: usize = 3000;
/// requestExperiences: PAGE_SIZE1 = EXP_URL_SEND_THRESHOLD / UUID_STR_LENGTH (37).
const PAGE_SIZE: usize = URL_SEND_THRESHOLD / 37;

/// One row of `experience_keys` (LLExperienceCache's fields).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExperienceDetails {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub properties: i32,
    /// SIM_ACCESS_PG / MATURE / ADULT (13, 21, 42).
    pub maturity: u8,
    pub owner: Uuid,
    pub group: Uuid,
    /// Where the experience says it happens (SLURL), possibly empty.
    pub slurl: String,
}

impl ExperienceDetails {
    /// What the cache keeps when the details could not be read: the
    /// question is still asked (requestExperiencesCoro builds a dummy
    /// entry, PROPERTY_INVALID, and calls the waiting slots).
    pub fn unknown(id: Uuid) -> Self {
        ExperienceDetails {
            id,
            properties: PROPERTY_INVALID,
            ..Default::default()
        }
    }

    pub fn grid_wide(&self) -> bool {
        self.properties & PROPERTY_GRID != 0
    }

    pub fn invalid(&self) -> bool {
        self.properties & PROPERTY_INVALID != 0
    }
}

/// The agent's permission for an experience (experiencePermissionResults).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperiencePermission {
    Allowed,
    Blocked,
    /// Neither: the experience asks again next time.
    Unset,
}

/// What `ExperiencePreferences` is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperiencePermissionChange {
    /// GET `?<id>`: read the permission.
    Query,
    /// PUT `{"<id>": {"permission": "Allow"}}`.
    Allow,
    /// PUT `{"<id>": {"permission": "Block"}}`.
    Block,
    /// DELETE `?<id>`.
    Forget,
}

/// The requests of a batch: GetExperienceInfo + `id/?page_size=81` and one
/// `public_id=` per experience, cut when the URL passes 3000 characters
/// (requestExperiences). Each URL comes with the experiences it asks for.
pub fn info_urls(base: &str, ids: &[Uuid]) -> Vec<(String, Vec<Uuid>)> {
    let mut base = base.to_owned();
    if !base.ends_with('/') {
        base.push('/');
    }
    base.push_str("id/");
    let start = format!("{base}?page_size={PAGE_SIZE}");
    let mut out = Vec::new();
    let mut url = start.clone();
    let mut batch = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        url.push_str(&format!("&public_id={id}"));
        batch.push(*id);
        if i + 1 == ids.len() || url.len() > URL_SEND_THRESHOLD {
            out.push((std::mem::replace(&mut url, start.clone()), std::mem::take(&mut batch)));
        }
    }
    out
}

/// requestExperiencesCoro: the rows of `experience_keys`; the ids of
/// `error_ids`, and the requested ones the answer leaves out, are unknown.
pub fn parse_info(reply: &Llsd, requested: &[Uuid]) -> Vec<ExperienceDetails> {
    let mut out: Vec<ExperienceDetails> = reply["experience_keys"]
        .as_array()
        .iter()
        .map(|row| ExperienceDetails {
            id: row["public_id"].as_uuid(),
            name: row["name"].to_string_value(),
            description: row["description"].to_string_value(),
            properties: row["properties"].as_i32(),
            maturity: u8::try_from(row["maturity"].as_i32()).unwrap_or(0),
            owner: row["agent_id"].as_uuid(),
            group: row["group_id"].as_uuid(),
            slurl: row["slurl"].to_string_value(),
        })
        .filter(|d| !d.id.is_nil())
        .collect();
    for id in requested {
        if !out.iter().any(|d| d.id == *id) {
            out.push(ExperienceDetails::unknown(*id));
        }
    }
    out
}

/// setExperiencePermission: `{"<id>": {"permission": "Allow" | "Block"}}`.
pub fn permission_body(experience: Uuid, permission: &str) -> Llsd {
    let mut map = std::collections::BTreeMap::new();
    map.insert(experience.to_string(), llsd_map! { "permission" => permission });
    Llsd::Map(map)
}

/// The URL of `ExperiencePreferences` for one experience: `?<id>` for a
/// read or a forget, the bare capability for a change.
pub fn preferences_url(cap: &str, experience: Uuid, change: ExperiencePermissionChange) -> String {
    match change {
        ExperiencePermissionChange::Allow | ExperiencePermissionChange::Block => cap.to_owned(),
        ExperiencePermissionChange::Query | ExperiencePermissionChange::Forget => format!("{cap}?{experience}"),
    }
}

/// experiencePermissionResults: an experience in `experiences` is allowed,
/// in `blocked` blocked, otherwise unset.
pub fn parse_permission(reply: &Llsd, experience: Uuid) -> ExperiencePermission {
    let listed = |name: &str| reply[name].as_array().iter().any(|v| v.as_uuid() == experience);
    if listed("experiences") {
        ExperiencePermission::Allowed
    } else if listed("blocked") {
        ExperiencePermission::Blocked
    } else {
        ExperiencePermission::Unset
    }
}

async fn llsd(request: reqwest::RequestBuilder) -> Result<Llsd, CapsError> {
    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        request.header("Accept", "application/llsd+xml").send(),
    )
    .await
    .map_err(|_| CapsError::Http("timeout".into()))?
    .map_err(|e| CapsError::Http(e.to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| CapsError::Http(e.to_string()))?;
    if bytes.is_empty() {
        return Ok(Llsd::Undef);
    }
    Ok(aurora_llsd::from_xml(&bytes)?)
}

/// Details of `ids`, one request per batch. A batch that fails gives
/// unknown experiences, so a question waiting for them is still shown.
pub async fn fetch_info(http: &reqwest::Client, base: Option<&str>, ids: &[Uuid]) -> Vec<ExperienceDetails> {
    let Some(base) = base else {
        return ids.iter().map(|id| ExperienceDetails::unknown(*id)).collect();
    };
    let mut out = Vec::new();
    for (url, batch) in info_urls(base, ids) {
        match llsd(http.get(&url)).await {
            Ok(reply) => out.extend(parse_info(&reply, &batch)),
            Err(e) => {
                log::warn!("GetExperienceInfo failed: {e}");
                out.extend(batch.iter().map(|id| ExperienceDetails::unknown(*id)));
            }
        }
    }
    out
}

/// LLExperienceCache::getGroupExperiencesCoro: the experiences a group
/// owns, from the `GroupExperiences` capability with the group as the whole
/// query (`?<group>`). A failure or a missing capability gives an empty
/// list, as the coroutine answers `fn(LLSD())`.
pub async fn fetch_group_experiences(http: &reqwest::Client, cap: Option<&str>, group: Uuid) -> Vec<Uuid> {
    let Some(cap) = cap else {
        log::warn!("No Group Experiences capability");
        return Vec::new();
    };
    match llsd(http.get(group_experiences_url(cap, group))).await {
        Ok(reply) => parse_group_experiences(&reply),
        Err(e) => {
            log::warn!("GroupExperiences failed: {e}");
            Vec::new()
        }
    }
}

pub fn group_experiences_url(cap: &str, group: Uuid) -> String {
    format!("{cap}?{group}")
}

/// The ids of `experience_ids`.
pub fn parse_group_experiences(reply: &Llsd) -> Vec<Uuid> {
    reply["experience_ids"]
        .as_array()
        .iter()
        .map(Llsd::as_uuid)
        .filter(|id| !id.is_nil())
        .collect()
}

/// Reads or changes the permission; the answer is the permission the
/// server now has (every reply carries the `experiences` / `blocked` lists).
pub async fn permission(
    http: &reqwest::Client,
    cap: Option<&str>,
    experience: Uuid,
    change: ExperiencePermissionChange,
) -> Result<ExperiencePermission, String> {
    let cap = cap.ok_or_else(|| "no ExperiencePreferences capability".to_owned())?;
    let url = preferences_url(cap, experience, change);
    let request = match change {
        ExperiencePermissionChange::Query => http.get(&url),
        ExperiencePermissionChange::Forget => http.delete(&url),
        ExperiencePermissionChange::Allow | ExperiencePermissionChange::Block => {
            let permission = if change == ExperiencePermissionChange::Allow {
                "Allow"
            } else {
                "Block"
            };
            http.put(&url)
                .header("Content-Type", "application/llsd+xml")
                .body(aurora_llsd::to_xml(&permission_body(experience, permission)))
        }
    };
    let reply = llsd(request).await.map_err(|e| e.to_string())?;
    Ok(parse_permission(&reply, experience))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_urls_ask_the_id_resource_in_batches_under_the_threshold() {
        let ids: Vec<Uuid> = (1..=200u128).map(Uuid::from_u128).collect();
        let urls = info_urls("https://cap.example/info", &ids);
        assert!(urls.len() > 1);
        let mut seen = Vec::new();
        for (url, batch) in &urls {
            assert!(url.starts_with("https://cap.example/info/id/?page_size=81&public_id="));
            // one more id than the threshold allows, like requestExperiences
            assert!(url.len() <= URL_SEND_THRESHOLD + "&public_id=".len() + 36);
            for id in batch {
                assert!(url.contains(&format!("public_id={id}")));
            }
            seen.extend(batch.iter().copied());
        }
        assert_eq!(seen, ids);
        let one = info_urls("https://cap.example/info/", &[Uuid::from_u128(7)]);
        assert_eq!(
            one,
            vec![(
                format!("https://cap.example/info/id/?page_size=81&public_id={}", Uuid::from_u128(7)),
                vec![Uuid::from_u128(7)]
            )]
        );
        assert!(info_urls("https://cap.example/info", &[]).is_empty());
    }

    #[test]
    fn info_rows_and_missing_experiences() {
        let known = Uuid::from_u128(1);
        let error = Uuid::from_u128(2);
        let silent = Uuid::from_u128(3);
        let reply = llsd_map! {
            "experience_keys" => Llsd::Array(vec![llsd_map! {
                "public_id" => known,
                "name" => "Club Teleport",
                "description" => "Doors of the club",
                "properties" => PROPERTY_GRID,
                "maturity" => 21,
                "agent_id" => Uuid::from_u128(9),
                "group_id" => Uuid::nil(),
                "slurl" => "secondlife://Club/1/2/3",
            }]),
            "error_ids" => Llsd::Array(vec![error.into()]),
        };
        let details = parse_info(&reply, &[known, error, silent]);
        let club = details.iter().find(|d| d.id == known).expect("known");
        assert_eq!(club.name, "Club Teleport");
        assert!(club.grid_wide() && !club.invalid());
        assert_eq!((club.maturity, club.owner), (21, Uuid::from_u128(9)));
        for id in [error, silent] {
            let d = details.iter().find(|d| d.id == id).expect("unknown kept");
            assert!(d.invalid() && !d.grid_wide() && d.name.is_empty());
        }
    }

    #[test]
    fn preference_requests_and_answers() {
        let id = Uuid::from_u128(0x42);
        let body = permission_body(id, "Block");
        assert_eq!(body[id.to_string().as_str()]["permission"].as_str(), "Block");
        assert_eq!(
            aurora_llsd::from_xml(&aurora_llsd::to_xml(&body)).expect("xml")[id.to_string().as_str()]["permission"].as_str(),
            "Block"
        );
        let cap = "https://cap.example/prefs";
        assert_eq!(preferences_url(cap, id, ExperiencePermissionChange::Block), cap);
        assert_eq!(preferences_url(cap, id, ExperiencePermissionChange::Allow), cap);
        assert_eq!(preferences_url(cap, id, ExperiencePermissionChange::Forget), format!("{cap}?{id}"));
        assert_eq!(preferences_url(cap, id, ExperiencePermissionChange::Query), format!("{cap}?{id}"));
        let other = Uuid::from_u128(0x43);
        let lists = llsd_map! {
            "experiences" => Llsd::Array(vec![other.into()]),
            "blocked" => Llsd::Array(vec![id.into()]),
        };
        assert_eq!(parse_permission(&lists, id), ExperiencePermission::Blocked);
        assert_eq!(parse_permission(&lists, other), ExperiencePermission::Allowed);
        assert_eq!(parse_permission(&lists, Uuid::from_u128(0x44)), ExperiencePermission::Unset);
        assert_eq!(parse_permission(&Llsd::Undef, id), ExperiencePermission::Unset);
    }

    #[test]
    fn group_experiences_ask_the_group_as_the_query() {
        let group = Uuid::from_u128(0x6);
        assert_eq!(
            group_experiences_url("https://cap.example/groupxp", group),
            format!("https://cap.example/groupxp?{group}")
        );
        let reply = llsd_map! {
            "experience_ids" => Llsd::Array(vec![Uuid::from_u128(1).into(), Uuid::nil().into(), Uuid::from_u128(2).into()]),
        };
        assert_eq!(parse_group_experiences(&reply), vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        assert!(parse_group_experiences(&Llsd::Undef).is_empty());
    }
}
