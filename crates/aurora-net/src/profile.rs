//! Avatar profiles: the AgentProfile capability reply and the legacy
//! AvatarPropertiesReply, as LLAvatarPropertiesProcessor reads them
//! (`newview/llavatarpropertiesprocessor.cpp` in Firestorm, originally
//! LGPL 2.1), plus the civil-date helpers of `LLDate` / `LLDateUtil`.

use aurora_llsd::Llsd;
use uuid::Uuid;

/// AVATAR_* flags of LLAvatarData (llavatarpropertiesprocessor.h).
pub mod flags {
    pub const ALLOW_PUBLISH: u32 = 1 << 0;
    pub const IDENTIFIED: u32 = 1 << 2;
    pub const TRANSACTED: u32 = 1 << 3;
    pub const ONLINE: u32 = 1 << 4;
}

/// A group shown in a profile.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileGroup {
    pub id: Uuid,
    pub name: String,
    pub insignia: Uuid,
}

/// Everything a profile shows (LLAvatarData).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AvatarProfile {
    pub id: Uuid,
    pub sl_image: Uuid,
    pub fl_image: Uuid,
    pub partner: Uuid,
    pub sl_about: String,
    pub fl_about: String,
    /// Registration date, seconds since the Unix epoch (UTC).
    pub born: Option<f64>,
    /// `hide_age`; None when the server does not support it
    /// (isHideAgeSupportedByServer).
    pub hide_age: Option<bool>,
    /// "lifetime", "secondlifetime_premium"... (badges).
    pub customer_type: String,
    /// Our private notes about this avatar (None: not sent).
    pub notes: Option<String>,
    /// Online status; None when the server did not say (AVATAR_ONLINE_UNDEFINED).
    pub online: Option<bool>,
    /// `flags` bits.
    pub flags: u32,
    /// Account type index: 0 resident, 1 trial, 2 charter member, 3 Linden.
    pub caption_index: u8,
    /// Special account caption, shown instead of the type ("El Jefe!").
    pub caption_text: String,
    pub groups: Vec<ProfileGroup>,
    /// Picks (id, name).
    pub picks: Vec<(Uuid, String)>,
    /// Came from AvatarPropertiesReply (no groups, picks or notes).
    pub legacy: bool,
}

impl AvatarProfile {
    pub fn allow_publish(&self) -> bool {
        self.flags & flags::ALLOW_PUBLISH != 0
    }
}

/// A pick (PickInfoReply / PickInfoUpdate).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PickInfo {
    pub id: Uuid,
    pub creator: Uuid,
    pub parcel: Uuid,
    pub name: String,
    pub desc: String,
    pub snapshot: Uuid,
    pub sim_name: String,
    /// Global position, meters.
    pub pos_global: glam::DVec3,
    pub sort_order: i32,
    pub enabled: bool,
}

/// CLASSIFIED_FLAG_* (llclassifiedflags.h).
pub mod classified_flags {
    pub const MATURE: u8 = 1 << 1;
    pub const QUERY_INC_MATURE: u8 = 1 << 3;
    pub const AUTO_RENEW: u8 = 1 << 5;
}

/// A classified ad (ClassifiedInfoReply).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClassifiedInfo {
    pub id: Uuid,
    pub creator: Uuid,
    /// Unix seconds.
    pub creation_date: u32,
    pub expiration_date: u32,
    pub category: u32,
    pub name: String,
    pub desc: String,
    pub parcel: Uuid,
    pub snapshot: Uuid,
    pub sim_name: String,
    pub pos_global: glam::DVec3,
    pub parcel_name: String,
    pub flags: u8,
    pub price: i32,
}

/// A parcel seen from afar (ParcelInfoReply, LLParcelData): picks of a
/// profile, place details of a place link.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParcelSummary {
    pub id: Uuid,
    pub owner: Uuid,
    pub name: String,
    pub desc: String,
    pub actual_area: i32,
    pub billable_area: i32,
    /// 0x1 mature region, 0x2 adult region (LLPanelPlaceProfile::processParcelInfo).
    pub flags: u8,
    pub sim_name: String,
    pub global: glam::DVec3,
    pub snapshot: Uuid,
    /// Traffic (dwell).
    pub dwell: f32,
    pub sale_price: i32,
    pub auction_id: i32,
}

/// LLPanelProfilePick::createLocationText: the non-empty parts joined with
/// ", ", then "  (x, y, z)" in region coordinates (two spaces, like LL).
pub fn location_text(parts: &[&str], pos: glam::DVec3) -> String {
    let mut s = parts.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(", ");
    if !s.is_empty() {
        s.push(' ');
    }
    if pos != glam::DVec3::ZERO {
        let (x, y, z) = (pos.x.round() as i64, pos.y.round() as i64, pos.z.round() as i64);
        s.push_str(&format!(" ({}, {}, {z})", x.rem_euclid(256), y.rem_euclid(256)));
    }
    s
}

/// GET <AgentProfile>/<id> reply (requestAvatarPropertiesCoro). None when
/// the reply is about another avatar than the one asked for.
pub fn parse_agent_profile(requested: Uuid, v: &Llsd) -> Option<AvatarProfile> {
    if v["id"].as_uuid() != requested {
        return None;
    }
    let mut flags = 0;
    for (key, bit) in [
        ("allow_publish", flags::ALLOW_PUBLISH),
        ("identified", flags::IDENTIFIED),
        ("transacted", flags::TRANSACTED),
    ] {
        if v[key].as_bool() {
            flags |= bit;
        }
    }
    let online = (!v["online"].is_undef()).then(|| v["online"].as_bool());
    if online == Some(true) {
        flags |= flags::ONLINE;
    }
    let born = match &v["member_since"] {
        Llsd::Date(d) => Some(*d),
        Llsd::String(s) => parse_iso_date(s),
        _ => None,
    };
    // "charter_member" won't be present if "caption" is set
    let (caption_index, caption_text) = if v.has("charter_member") {
        (v["charter_member"].as_i32().clamp(0, 255) as u8, String::new())
    } else {
        (0, v["caption"].to_string_value())
    };
    Some(AvatarProfile {
        id: requested,
        sl_image: v["sl_image_id"].as_uuid(),
        fl_image: v["fl_image_id"].as_uuid(),
        partner: v["partner_id"].as_uuid(),
        sl_about: v["sl_about_text"].to_string_value(),
        fl_about: v["fl_about_text"].to_string_value(),
        born,
        hide_age: v.has("hide_age").then(|| v["hide_age"].as_bool()),
        customer_type: v["customer_type"].to_string_value(),
        notes: v.has("notes").then(|| v["notes"].to_string_value()),
        online,
        flags,
        caption_index,
        caption_text,
        groups: v["groups"]
            .as_array()
            .iter()
            .map(|g| ProfileGroup {
                id: g["id"].as_uuid(),
                name: g["name"].to_string_value(),
                insignia: g["image_id"].as_uuid(),
            })
            .filter(|g| !g.id.is_nil())
            .collect(),
        picks: v["picks"]
            .as_array()
            .iter()
            .map(|p| (p["id"].as_uuid(), p["name"].to_string_value()))
            .filter(|p| !p.0.is_nil())
            .collect(),
        legacy: false,
    })
}

/// CharterMember of AvatarPropertiesReply: one byte is the account type
/// index, a longer field the special caption (processAvatarPropertiesReply).
pub fn parse_charter_member(field: &[u8]) -> (u8, String) {
    match field.len() {
        0 => (0, String::new()),
        1 => (field[0], String::new()),
        _ => (0, aurora_msg::field_str(field)),
    }
}

/// "mm/dd/yyyy" in Pacific time, as BornOn is sent
/// (LLDateUtil::dateFromPDTString: midnight + 8 hours).
pub fn parse_pdt_date(s: &str) -> Option<f64> {
    let mut it = s.trim().split('/').map(|p| p.trim().parse::<i64>());
    let (m, d, y) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((days_from_civil(y, m as u32, d as u32) * 86_400 + 8 * 3600) as f64)
}

/// "yyyy-mm-dd..." (an ISO date sent as a string).
fn parse_iso_date(s: &str) -> Option<f64> {
    let mut it = s.get(..10)?.split('-').map(|p| p.parse::<i64>());
    let (y, m, d) = (it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some((days_from_civil(y, m as u32, d as u32) * 86_400) as f64)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (H. Hinnant).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) of a day count since 1970-01-01 (LLDate::split, UTC).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    #[test]
    fn civil_round_trip() {
        for days in [-1000, 0, 11_016, 19_000, 20_735] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn pdt_date() {
        let t = parse_pdt_date("05/12/2007").expect("valid date");
        assert_eq!(civil_from_days((t / 86_400.0) as i64), (2007, 5, 12));
        assert_eq!(t as i64 % 86_400, 8 * 3600);
        assert!(parse_pdt_date("2007-05-12").is_none());
        assert!(parse_pdt_date("13/01/2007").is_none());
    }

    #[test]
    fn agent_profile_reply() {
        let id = Uuid::from_u128(7);
        let v = llsd_map! {
            "id" => Llsd::Uuid(id),
            "sl_about_text" => "Bonjour",
            "member_since" => Llsd::Date(1_178_928_000.0),
            "hide_age" => false,
            "online" => true,
            "transacted" => true,
            "charter_member" => 2,
            "notes" => "ma note",
            "groups" => Llsd::Array(vec![llsd_map! { "id" => Llsd::Uuid(Uuid::from_u128(9)), "name" => "Loups" }]),
            "picks" => Llsd::Array(vec![llsd_map! { "id" => Llsd::Uuid(Uuid::from_u128(3)), "name" => "Plage" }]),
        };
        let p = parse_agent_profile(id, &v).expect("same avatar");
        assert_eq!(p.sl_about, "Bonjour");
        assert_eq!(p.born, Some(1_178_928_000.0));
        assert_eq!(p.hide_age, Some(false));
        assert_eq!(p.online, Some(true));
        assert_eq!(p.flags, flags::TRANSACTED | flags::ONLINE);
        assert_eq!(p.caption_index, 2);
        assert_eq!(p.notes.as_deref(), Some("ma note"));
        assert_eq!(p.groups[0].name, "Loups");
        assert_eq!(p.picks, vec![(Uuid::from_u128(3), "Plage".to_owned())]);
        // no "online" key: unknown; no "hide_age": not supported
        let p = parse_agent_profile(id, &llsd_map! { "id" => Llsd::Uuid(id), "caption" => "El Jefe!" }).expect("same avatar");
        assert_eq!((p.online, p.hide_age), (None, None));
        assert_eq!(p.caption_text, "El Jefe!");
        // reply about someone else
        assert!(parse_agent_profile(Uuid::from_u128(8), &v).is_none());
    }

    #[test]
    fn location() {
        let p = glam::DVec3::new(256_000.0 + 128.4, 512.0 + 64.0, 22.6);
        assert_eq!(location_text(&["", "Plage", "Aurora"], p), "Plage, Aurora  (128, 64, 23)");
        assert_eq!(location_text(&["Plage"], glam::DVec3::ZERO), "Plage ");
    }

    #[test]
    fn charter_member_field() {
        assert_eq!(parse_charter_member(&[3]), (3, String::new()));
        assert_eq!(parse_charter_member(b"El Jefe!\0"), (0, "El Jefe!".to_owned()));
    }
}
