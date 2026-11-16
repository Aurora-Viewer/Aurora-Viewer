//! Experiences: their details and the agent's permission for each of them,
//! like LLExperienceCache (indra/llmessage/llexperiencecache.cpp, Firestorm,
//! originally LGPL 2.1). `GetExperienceInfo` gives the details of a batch
//! of experiences (requestExperiences, requestExperiencesCoro);
//! `ExperiencePreferences` reads, sets (Allow / Block) and forgets the
//! agent's permission (getExperiencePermission, setExperiencePermission,
//! forgetExperiencePermission). `IsExperienceAdmin` and `UpdateExperience`
//! let an administrator edit the profile (getExperienceAdmin,
//! updateExperience, LLFloaterExperienceProfile::updatePackage).

use crate::caps::CapsError;
use aurora_llsd::{Llsd, llsd_map};
use uuid::Uuid;

/// LLExperienceCache::PROPERTY_INVALID: no such experience, or its details
/// could not be read.
pub const PROPERTY_INVALID: i32 = 1 << 0;
/// LLExperienceCache::PROPERTY_PRIVILEGED: an experience the grid runs.
pub const PROPERTY_PRIVILEGED: i32 = 1 << 3;
/// LLExperienceCache::PROPERTY_GRID: a grid-wide experience (else land-scope).
pub const PROPERTY_GRID: i32 = 1 << 4;
/// LLExperienceCache::PROPERTY_PRIVATE: "Hide In Search".
pub const PROPERTY_PRIVATE: i32 = 1 << 5;
/// LLExperienceCache::PROPERTY_DISABLED: "Enable Experience" cleared.
pub const PROPERTY_DISABLED: i32 = 1 << 6;
/// The ratings an experience can have (the Edit tab's combo): General,
/// Moderate, Adult.
pub const MATURITIES: [u8; 3] = [13, 21, 42];

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
    /// `extended_metadata`'s marketplace address, possibly empty.
    pub marketplace: String,
    /// `extended_metadata`'s logo texture, nil for none.
    pub logo: Uuid,
    /// The row as the server gave it: an edit sends it back with its
    /// changes (LLFloaterExperienceProfile::mPackage).
    pub package: Llsd,
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

    /// The row of `experience_keys` (refreshExperience's fields).
    pub fn from_row(row: &Llsd) -> Self {
        let (marketplace, logo) = parse_metadata(&row["extended_metadata"].to_string_value());
        ExperienceDetails {
            id: row["public_id"].as_uuid(),
            name: row["name"].to_string_value(),
            description: row["description"].to_string_value(),
            properties: row["properties"].as_i32(),
            maturity: u8::try_from(row["maturity"].as_i32()).unwrap_or(0),
            owner: row["agent_id"].as_uuid(),
            group: row["group_id"].as_uuid(),
            slurl: row["slurl"].to_string_value(),
            marketplace,
            logo,
            package: row.clone(),
        }
    }
}

/// refreshExperience: `extended_metadata` is an LLSD XML document of its
/// own, `{"marketplace": text, "logo": uuid}`; an unreadable one is empty.
pub fn parse_metadata(xml: &str) -> (String, Uuid) {
    if xml.trim().is_empty() {
        return (String::new(), Uuid::nil());
    }
    match aurora_llsd::from_xml(xml.as_bytes()) {
        Ok(m) => (m["marketplace"].to_string_value(), m["logo"].as_uuid()),
        Err(_) => (String::new(), Uuid::nil()),
    }
}

/// updatePackage: the metadata as LLSDXMLFormatter writes it (`<llsd>`
/// without an XML declaration).
pub fn metadata_xml(marketplace: &str, logo: Uuid) -> String {
    let xml = aurora_llsd::to_xml_string(&llsd_map! { "logo" => logo, "marketplace" => marketplace });
    xml.trim_start_matches("<?xml version=\"1.0\" ?>").to_owned()
}

/// setMaturityString: the combo's rating for any maturity (up to General,
/// up to Moderate, else Adult).
pub fn maturity_choice(maturity: u8) -> u8 {
    MATURITIES.into_iter().find(|m| maturity <= *m).unwrap_or(42)
}

/// What the profile's Edit tab changes (updatePackage).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExperienceEdit {
    pub name: String,
    pub description: String,
    /// Empty: no location ("Clear Location").
    pub slurl: String,
    pub maturity: u8,
    pub marketplace: String,
    pub logo: Uuid,
    pub group: Uuid,
    /// PROPERTY_DISABLED cleared.
    pub enabled: bool,
    /// PROPERTY_PRIVATE.
    pub private: bool,
}

impl ExperienceEdit {
    /// The fields as the details have them (refreshExperience fills the
    /// Edit tab).
    pub fn of(d: &ExperienceDetails) -> Self {
        ExperienceEdit {
            name: d.name.clone(),
            description: d.description.clone(),
            slurl: d.slurl.clone(),
            maturity: maturity_choice(d.maturity),
            marketplace: d.marketplace.clone(),
            logo: d.logo,
            group: d.group,
            enabled: d.properties & PROPERTY_DISABLED == 0,
            private: d.properties & PROPERTY_PRIVATE != 0,
        }
    }

    /// onFieldChanged: Save is enabled once a field differs.
    pub fn dirty(&self, d: &ExperienceDetails) -> bool {
        *self != ExperienceEdit::of(d)
    }
}

/// LLExperienceCache::updateExperienceCoro: the row the server gave with
/// the edit's fields, without `quota`, `expiration` and `agent_id`.
pub fn update_body(d: &ExperienceDetails, edit: &ExperienceEdit) -> Llsd {
    let mut map = d.package.as_map().cloned().unwrap_or_default();
    map.insert("public_id".into(), Llsd::Uuid(d.id));
    map.insert("name".into(), edit.name.clone().into());
    map.insert("description".into(), edit.description.clone().into());
    map.insert("slurl".into(), edit.slurl.clone().into());
    map.insert("maturity".into(), Llsd::Integer(edit.maturity.into()));
    map.insert("extended_metadata".into(), metadata_xml(&edit.marketplace, edit.logo).into());
    let mut properties = d.properties;
    if edit.enabled {
        properties &= !PROPERTY_DISABLED;
    } else {
        properties |= PROPERTY_DISABLED;
    }
    if edit.private {
        properties |= PROPERTY_PRIVATE;
    } else {
        properties &= !PROPERTY_PRIVATE;
    }
    map.insert("properties".into(), Llsd::Integer(properties));
    map.insert("group_id".into(), Llsd::Uuid(edit.group));
    for key in ["quota", "expiration", "agent_id"] {
        map.remove(key);
    }
    Llsd::Map(map)
}

/// A field the server ignored (onSaveComplete's `removed`): its
/// `error_tag` picks the message, else the server's English text.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateNote {
    pub field: String,
    pub tag: String,
    pub extra: String,
    pub message: String,
}

/// UpdateExperience's answer: the ignored fields, and the experience as
/// saved when the answer names it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExperienceUpdate {
    pub notes: Vec<UpdateNote>,
    pub details: Option<ExperienceDetails>,
}

/// onSaveComplete: `removed` (without `public_id`, which the API reports
/// for nothing) and the first row of `experience_keys` when it is ours.
pub fn parse_update(reply: &Llsd, experience: Uuid) -> ExperienceUpdate {
    let notes = reply["removed"]
        .as_map()
        .map(|removed| {
            removed
                .iter()
                .filter(|(field, _)| field.as_str() != "public_id")
                .map(|(field, data)| UpdateNote {
                    field: field.clone(),
                    tag: data["error_tag"].to_string_value(),
                    extra: data["extra_info"].to_string_value(),
                    message: data["en"].to_string_value(),
                })
                .collect()
        })
        .unwrap_or_default();
    let details = reply["experience_keys"]
        .as_array()
        .first()
        .filter(|row| row["public_id"].as_uuid() == experience)
        .map(ExperienceDetails::from_row);
    ExperienceUpdate { notes, details }
}

/// getExperienceAdminCoro: `?experience_id=<id>`.
pub fn admin_url(cap: &str, experience: Uuid) -> String {
    format!("{cap}?experience_id={experience}")
}

/// experienceIsAdmin: `status` is true for an administrator.
pub fn parse_admin(reply: &Llsd) -> bool {
    reply["status"].as_bool()
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
        .map(ExperienceDetails::from_row)
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
    .map_err(|e| CapsError::Http(e.without_url().to_string()))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(CapsError::Status(status.as_u16()));
    }
    let bytes = resp.bytes().await.map_err(|e| CapsError::Http(e.without_url().to_string()))?;
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

/// The tabs of the "My Experiences" window (LLFloaterExperiences::postBuild).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExperienceListKind {
    Allowed,
    Blocked,
    Admin,
    Contributor,
    Owned,
}

impl ExperienceListKind {
    /// retrieveExperienceListCoro's tab map: `GetExperiences` answers
    /// `experiences` and `blocked`, the other capabilities `experience_ids`.
    pub fn key(self) -> &'static str {
        match self {
            ExperienceListKind::Allowed => "experiences",
            ExperienceListKind::Blocked => "blocked",
            _ => "experience_ids",
        }
    }
}

/// The ids under the key of each of `kinds` in one answer.
pub fn parse_list(reply: &Llsd, kind: ExperienceListKind) -> Vec<Uuid> {
    reply[kind.key()]
        .as_array()
        .iter()
        .map(Llsd::as_uuid)
        .filter(|id| !id.is_nil())
        .collect()
}

/// One capability's answer for the tabs it feeds; a failure or a missing
/// capability gives `None` for each of them (the tab shows no experience).
pub async fn fetch_lists(
    http: &reqwest::Client,
    cap: Option<&str>,
    kinds: &[ExperienceListKind],
) -> Vec<(ExperienceListKind, Option<Vec<Uuid>>)> {
    let reply = match cap {
        Some(cap) => llsd(http.get(cap))
            .await
            .map_err(|e| log::warn!("experience list failed: {e}"))
            .ok(),
        None => None,
    };
    kinds
        .iter()
        .map(|kind| (*kind, reply.as_ref().map(|r| parse_list(r, *kind))))
        .collect()
}

/// experienceIsAdmin: whether the agent administers the experience; a
/// failure or a missing capability is no.
pub async fn is_admin(http: &reqwest::Client, cap: Option<&str>, experience: Uuid) -> bool {
    let Some(cap) = cap else {
        return false;
    };
    match llsd(http.get(admin_url(cap, experience))).await {
        Ok(reply) => parse_admin(&reply),
        Err(e) => {
            log::warn!("IsExperienceAdmin failed: {e}");
            false
        }
    }
}

/// updateExperienceCoro: POST the package, read what was saved.
pub async fn update(http: &reqwest::Client, cap: Option<&str>, experience: Uuid, body: &Llsd) -> Result<ExperienceUpdate, String> {
    let cap = cap.ok_or_else(|| "no UpdateExperience capability".to_owned())?;
    let request = http
        .post(cap)
        .header("Content-Type", "application/llsd+xml")
        .body(aurora_llsd::to_xml(body));
    let reply = llsd(request).await.map_err(|e| e.to_string())?;
    Ok(parse_update(&reply, experience))
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
    fn lists_are_read_under_the_key_of_their_tab() {
        let a = Uuid::from_u128(1);
        let b = Uuid::from_u128(2);
        let reply = llsd_map! {
            "experiences" => Llsd::Array(vec![Llsd::Uuid(a), Llsd::Uuid(Uuid::nil())]),
            "blocked" => Llsd::Array(vec![Llsd::Uuid(b)]),
        };
        assert_eq!(parse_list(&reply, ExperienceListKind::Allowed), vec![a]);
        assert_eq!(parse_list(&reply, ExperienceListKind::Blocked), vec![b]);
        assert!(parse_list(&reply, ExperienceListKind::Owned).is_empty());
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

    fn owned_row(id: Uuid) -> Llsd {
        llsd_map! {
            "public_id" => id,
            "name" => "Club Teleport",
            "description" => "Doors of the club",
            "properties" => PROPERTY_GRID | PROPERTY_PRIVATE,
            "maturity" => 21,
            "agent_id" => Uuid::from_u128(9),
            "group_id" => Uuid::nil(),
            "slurl" => "",
            "quota" => 5,
            "expiration" => 3600,
            "extended_metadata" => metadata_xml("https://market.example/club", Uuid::from_u128(0x10)),
        }
    }

    #[test]
    fn metadata_and_rows_keep_what_an_edit_sends_back() {
        let id = Uuid::from_u128(1);
        let d = ExperienceDetails::from_row(&owned_row(id));
        assert_eq!(d.marketplace, "https://market.example/club");
        assert_eq!(d.logo, Uuid::from_u128(0x10));
        assert!(metadata_xml("", Uuid::nil()).starts_with("<llsd>"));
        assert_eq!(parse_metadata("not xml"), (String::new(), Uuid::nil()));
        assert_eq!(parse_metadata(""), (String::new(), Uuid::nil()));
        let edit = ExperienceEdit::of(&d);
        assert!(edit.enabled && edit.private && !edit.dirty(&d));
        assert_eq!(
            (maturity_choice(0), maturity_choice(13), maturity_choice(20), maturity_choice(99)),
            (13, 13, 21, 42)
        );
    }

    #[test]
    fn an_update_sends_the_row_with_its_changes_and_without_the_owner_fields() {
        let id = Uuid::from_u128(1);
        let d = ExperienceDetails::from_row(&owned_row(id));
        let mut edit = ExperienceEdit::of(&d);
        edit.name = "Club Doors".into();
        edit.enabled = false;
        edit.private = false;
        edit.maturity = 42;
        edit.slurl = "http://maps.secondlife.com/secondlife/Plaza/1/2/3".into();
        edit.logo = Uuid::nil();
        edit.group = Uuid::from_u128(0x6);
        assert!(edit.dirty(&d));
        let body = update_body(&d, &edit);
        for gone in ["quota", "expiration", "agent_id"] {
            assert!(body[gone].is_undef(), "{gone} is not sent");
        }
        assert_eq!(body["public_id"].as_uuid(), id);
        assert_eq!(body["name"].as_str(), "Club Doors");
        assert_eq!(body["description"].as_str(), "Doors of the club");
        assert_eq!(body["maturity"].as_i32(), 42);
        assert_eq!(body["group_id"].as_uuid(), Uuid::from_u128(0x6));
        assert_eq!(body["properties"].as_i32(), PROPERTY_GRID | PROPERTY_DISABLED);
        assert_eq!(
            parse_metadata(body["extended_metadata"].as_str()),
            ("https://market.example/club".to_owned(), Uuid::nil())
        );
        // an experience read before its row (no package) still names itself
        let bare = update_body(&ExperienceDetails::unknown(id), &edit);
        assert_eq!(bare["public_id"].as_uuid(), id);
    }

    #[test]
    fn admin_and_update_answers() {
        let id = Uuid::from_u128(1);
        assert_eq!(
            admin_url("https://cap.example/admin", id),
            format!("https://cap.example/admin?experience_id={id}")
        );
        assert!(parse_admin(&llsd_map! { "status" => true }));
        assert!(!parse_admin(&llsd_map! { "status" => false }));
        assert!(!parse_admin(&Llsd::Undef));
        let reply = llsd_map! {
            "removed" => llsd_map! {
                "public_id" => llsd_map! { "error_tag" => "Uneditable" },
                "group_id" => llsd_map! { "error_tag" => "NotInGroup", "en" => "not in group" },
            },
            "experience_keys" => Llsd::Array(vec![owned_row(id)]),
        };
        let update = parse_update(&reply, id);
        assert_eq!(update.notes.len(), 1);
        assert_eq!(
            (update.notes[0].field.as_str(), update.notes[0].tag.as_str()),
            ("group_id", "NotInGroup")
        );
        assert_eq!(update.details.map(|d| d.name), Some("Club Teleport".into()));
        // another experience's row refreshes nothing
        assert!(parse_update(&reply, Uuid::from_u128(2)).details.is_none());
        assert_eq!(parse_update(&Llsd::Undef, id), ExperienceUpdate::default());
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
