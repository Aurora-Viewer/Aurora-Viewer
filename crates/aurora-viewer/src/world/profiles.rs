//! Avatar profiles: cache of the replies, requests, local edits, and the
//! texts of the profile window (LLPanelProfileSecondLife::fillAgeData /
//! fillAccountStatus / processOnlineStatus, LLDateUtil::ageFromDate,
//! LLAvatarPropertiesProcessor::accountType / paymentInfo in
//! `newview/llpanelprofile.cpp`, `lldateutil.cpp`,
//! `llavatarpropertiesprocessor.cpp`, originally LGPL 2.1).

use aurora_llsd::Llsd;
use aurora_net::profile::{civil_from_days, days_from_civil, flags};
use aurora_net::{AvatarProfile, ClassifiedInfo, ParcelSummary, PickInfo};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

#[derive(Default)]
pub struct Profiles {
    map: HashMap<Uuid, AvatarProfile>,
    pending: HashSet<Uuid>,
    requested: HashSet<Uuid>,
    /// Requests sent whose reply has not arrived yet.
    in_flight: HashSet<Uuid>,
    /// Pick details (PickInfoReply), by pick id.
    pub picks: HashMap<Uuid, PickInfo>,
    /// Classifieds of each avatar (AvatarClassifiedReply).
    pub classified_lists: HashMap<Uuid, Vec<(Uuid, String)>>,
    pub classifieds: HashMap<Uuid, ClassifiedInfo>,
    /// Clicks on our classifieds (SearchStatRequest).
    pub classified_stats: HashMap<Uuid, aurora_net::profile::ClassifiedStats>,
    /// Parcels of the picks and classifieds (ParcelInfoReply).
    pub parcels: HashMap<Uuid, ParcelSummary>,
}

impl Profiles {
    pub fn get(&self, id: &Uuid) -> Option<&AvatarProfile> {
        self.map.get(id)
    }

    /// Ask for a profile once (pictures of the conversations).
    pub fn want(&mut self, id: Uuid) {
        if !id.is_nil() && !self.map.contains_key(&id) && !self.requested.contains(&id) {
            self.pending.insert(id);
        }
    }

    /// Ask again, even if known: the profile window reloads its data each
    /// time it opens (LLPanelProfile::updateData).
    pub fn refresh(&mut self, id: Uuid) {
        if !id.is_nil() {
            self.requested.remove(&id);
            self.pending.insert(id);
        }
    }

    pub fn take_requests(&mut self) -> Vec<Uuid> {
        let v: Vec<Uuid> = self.pending.drain().collect();
        self.requested.extend(v.iter().copied());
        self.in_flight.extend(v.iter().copied());
        v
    }

    /// A request for this profile (`want` / `refresh`) is still unanswered:
    /// what is known of it may be out of date.
    pub fn awaiting(&self, id: &Uuid) -> bool {
        self.pending.contains(id) || self.in_flight.contains(id)
    }

    pub fn insert(&mut self, p: AvatarProfile) {
        self.in_flight.remove(&p.id);
        let old = self.map.get(&p.id);
        // the legacy reply has no notes: keep the ones we know
        let notes = p.notes.clone().or_else(|| old.and_then(|o| o.notes.clone()));
        self.map.insert(p.id, AvatarProfile { notes, ..p });
    }

    /// One of our picks saved (or created): its name in our profile too.
    pub fn pick_saved(&mut self, owner: Uuid, pick: PickInfo) {
        if let Some(p) = self.map.get_mut(&owner) {
            match p.picks.iter_mut().find(|(id, _)| *id == pick.id) {
                Some(e) => e.1 = pick.name.clone(),
                None => p.picks.push((pick.id, pick.name.clone())),
            }
        }
        self.picks.insert(pick.id, pick);
    }

    pub fn pick_deleted(&mut self, owner: Uuid, id: Uuid) {
        if let Some(p) = self.map.get_mut(&owner) {
            p.picks.retain(|(pid, _)| *pid != id);
        }
        self.picks.remove(&id);
    }

    /// One of our classifieds saved or published: shown at once, at the end
    /// of our list when it is new.
    pub fn classified_saved(&mut self, owner: Uuid, c: ClassifiedInfo) {
        let list = self.classified_lists.entry(owner).or_default();
        match list.iter_mut().find(|(id, _)| *id == c.id) {
            Some(e) => e.1 = c.name.clone(),
            None => list.push((c.id, c.name.clone())),
        }
        self.classifieds.insert(c.id, c);
    }

    pub fn classified_deleted(&mut self, owner: Uuid, id: Uuid) {
        if let Some(l) = self.classified_lists.get_mut(&owner) {
            l.retain(|(cid, _)| *cid != id);
        }
        self.classifieds.remove(&id);
    }

    /// A saved change, shown at once (the viewer does not read the profile
    /// back after a PUT). `notes` belong to `target`; the other keys to us.
    pub fn apply_local(&mut self, target: Uuid, data: &Llsd) {
        let Some(p) = self.map.get_mut(&target) else {
            return;
        };
        for (key, v) in data.as_map().into_iter().flatten() {
            match key.as_str() {
                "sl_about_text" => p.sl_about = v.to_string_value(),
                "fl_about_text" => p.fl_about = v.to_string_value(),
                "notes" => p.notes = Some(v.to_string_value()),
                "hide_age" => p.hide_age = Some(v.as_bool()),
                // the pictures shown from it (name plates, lists, the login
                // screen) follow (LLAvatarIconIDCache::add once saved)
                "sl_image_id" => p.sl_image = v.as_uuid(),
                "fl_image_id" => p.fl_image = v.as_uuid(),
                "allow_publish" => {
                    p.flags = if v.as_bool() {
                        p.flags | flags::ALLOW_PUBLISH
                    } else {
                        p.flags & !flags::ALLOW_PUBLISH
                    };
                }
                _ => {}
            }
        }
    }

    /// A group's insignia changed: the group lists of the profiles known
    /// show the new one.
    pub fn group_insignia(&mut self, group: Uuid, insignia: Uuid) {
        for p in self.map.values_mut() {
            for g in p.groups.iter_mut().filter(|g| g.id == group) {
                g.insignia = insignia;
            }
        }
    }
}

/// Classified ad texts (LLPanelProfileClassified::processProperties).
pub fn classified_maturity(flags: u8) -> &'static str {
    use aurora_net::profile::classified_flags::{MATURE, QUERY_INC_MATURE};
    if flags & (MATURE | QUERY_INC_MATURE) != 0 {
        aurora_i18n::tr!("common-moderate")
    } else {
        aurora_i18n::tr!("world-profiles-general-content")
    }
}

/// "mm/dd/yyyy" in SLT (date_fmt of panel_profile_classified.xml).
pub fn slt_date(secs: u32) -> String {
    let secs = i64::from(secs);
    // SLT is US Pacific time, with its daylight saving rule
    let off = crate::world::chat_log::format::pacific_offset_hours(secs);
    let (y, m, d) = civil_from_days((secs + off * 3600).div_euclid(86_400));
    aurora_i18n::tr!(
        "profile-date",
        month = format!("{m:02}"),
        day = format!("{d:02}"),
        year = y.to_string()
    )
}

fn days_in_month(y: i64, m: u32) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)
}

fn split(secs: f64) -> (i64, u32, u32) {
    civil_from_days((secs / 86_400.0).floor() as i64)
}

/// LLDateUtil::ageFromDate with Firestorm's total days ("TotalDaysOld"):
/// "17 years 4 months; 6350 days", "3 weeks; 23 days", "5 days".
fn age_from_date(language: &str, born: f64, now: f64) -> String {
    let (age, with_total) = age_short(language, born, now);
    if !with_total {
        return age;
    }
    let total = ((now - born) / 86_400.0) as i64;
    let total = aurora_i18n::tr_in!(language, "profile-age-days", count = total);
    aurora_i18n::tr_in!(language, "profile-age-total", age = age, days = total)
}

/// The age of LLDateUtil::ageFromDate ("17 years 4 months", "3 weeks",
/// "5 days"), and whether Firestorm follows it with the total days.
fn age_short(language: &str, born: f64, now: f64) -> (String, bool) {
    let (by, bm, bd) = split(born);
    let (mut ny, mut nm, nd) = split(now);
    // grade-school subtraction, borrowing from the left
    let mut days = nd as i64 - bd as i64;
    if days < 0 {
        if nm == 1 {
            ny -= 1;
            nm = 12;
        } else {
            nm -= 1;
        }
        days += days_in_month(ny, nm);
    }
    let mut months = nm as i64 - bm as i64;
    if months < 0 {
        ny -= 1;
        months += 12;
    }
    let years = ny - by;
    if months > 0 || years > 0 {
        let y = aurora_i18n::tr_in!(language, "profile-age-years", count = years);
        let m = aurora_i18n::tr_in!(language, "profile-age-months", count = months);
        let age = match (years > 0, months > 0) {
            (true, true) => aurora_i18n::tr_in!(language, "profile-age-parts", years = y, months = m),
            (true, false) => y,
            _ => m,
        };
        return (age, true);
    }
    let weeks = days / 7;
    if weeks > 0 {
        return (aurora_i18n::tr_in!(language, "profile-age-weeks", count = weeks), true);
    }
    if days % 7 > 0 {
        return (aurora_i18n::tr_in!(language, "profile-age-days", count = days % 7), false);
    }
    (aurora_i18n::tr_in!(language, "world-profiles-joined-today").to_owned(), false)
}

/// The birth line of the Aurora profile's header: the date, and the age
/// without its total days (those are in the tooltip, `birth_text`). Only
/// the day and the month, without an age, when someone hides their age.
pub fn birth_line(p: &AvatarProfile, own: bool, now: f64) -> Option<String> {
    birth_line_in(aurora_i18n::language(), p, own, now)
}

fn birth_line_in(language: &str, p: &AvatarProfile, own: bool, now: f64) -> Option<String> {
    let born = p.born?;
    let (y, m, d) = split(born);
    if !own && p.hide_age == Some(true) {
        return Some(aurora_i18n::tr_in!(
            language,
            "profile-date-hidden",
            day = format!("{d:02}"),
            month = format!("{m:02}")
        ));
    }
    let date = aurora_i18n::tr_in!(
        language,
        "profile-date",
        day = format!("{d:02}"),
        month = format!("{m:02}"),
        year = y.to_string()
    );
    Some(format!("{date} · {}", age_short(language, born, now).0))
}

/// fillAgeData: localized birth date and age; omit year when someone hides age.
pub fn birth_text(p: &AvatarProfile, own: bool, now: f64) -> Option<String> {
    birth_text_in(aurora_i18n::language(), p, own, now)
}

fn birth_text_in(language: &str, p: &AvatarProfile, own: bool, now: f64) -> Option<String> {
    let born = p.born?;
    let (y, m, d) = split(born);
    if !own && p.hide_age == Some(true) {
        return Some(aurora_i18n::tr_in!(
            language,
            "profile-date-hidden",
            day = format!("{d:02}"),
            month = format!("{m:02}")
        ));
    }
    let date = aurora_i18n::tr_in!(
        language,
        "profile-date",
        day = format!("{d:02}"),
        month = format!("{m:02}"),
        year = y.to_string()
    );
    Some(aurora_i18n::tr_in!(
        language,
        "profile-birth-age",
        date = date,
        age = age_from_date(language, born, now)
    ))
}

/// "Account" (CaptionTextAcctInfo): account type, then the payment info.
pub fn account_text(p: &AvatarProfile) -> String {
    if !p.caption_text.is_empty() {
        // special accounts (M Linden) show their caption, without payment info
        return p.caption_text.clone();
    }
    let kind = [
        aurora_i18n::tr!("common-resident"),
        aurora_i18n::tr!("world-profiles-test"),
        aurora_i18n::tr!("world-profiles-charter-member"),
        aurora_i18n::tr!("world-profiles-linden-lab-employee"),
    ][p.caption_index.min(3) as usize];
    if p.caption_index >= 3 {
        return kind.to_owned();
    }
    let payment = if p.flags & flags::TRANSACTED != 0 {
        aurora_i18n::tr!("world-profiles-payment-info-used")
    } else if p.flags & flags::IDENTIFIED != 0 {
        aurora_i18n::tr!("world-profiles-payment-info-on-file")
    } else {
        aurora_i18n::tr!("world-profiles-no-payment-info-on-file")
    };
    format!("{kind}\n{payment}")
}

/// LLAvatarPropertiesProcessor::hasPaymentInfoOnFile: special accounts and
/// Linden employees do not reveal theirs and count as having one.
pub fn payment_info_on_file(p: &AvatarProfile) -> bool {
    !p.caption_text.is_empty() || p.caption_index == 3 || p.flags & (flags::TRANSACTED | flags::IDENTIFIED) != 0
}

/// Account badge next to "Account" (LLPanelProfileSecondLife::fillAccountStatus):
/// the first that applies.
pub fn badge(p: &AvatarProfile) -> Option<&'static str> {
    // the original beta testers registered before 2003-06-23
    let beta_end = (days_from_civil(2003, 6, 23) * 86_400) as f64;
    if p.caption_index == 3 {
        return Some(aurora_i18n::tr!("world-profiles-linden-lab-employee"));
    }
    if p.born.is_some_and(|b| b < beta_end) {
        return Some(aurora_i18n::tr!("world-profiles-original-beta-tester"));
    }
    match p.customer_type.to_lowercase().as_str() {
        "beta_lifetime" => Some(aurora_i18n::tr!("world-profiles-lifetime-beta-member")),
        "lifetime" => Some(aurora_i18n::tr!("world-profiles-lifetime-member")),
        "secondlifetime_premium" => Some(aurora_i18n::tr!("world-profiles-lifetime-premium")),
        "secondlifetime_premium_plus" => Some(aurora_i18n::tr!("world-profiles-lifetime-premium-plus")),
        _ => None,
    }
}

/// What the AgentProfile reply says of a paid membership ("Premium",
/// "Premium_Plus"); the lifetime ones are Firestorm's badges.
fn premium(p: &AvatarProfile) -> Option<&'static str> {
    match p.customer_type.to_lowercase().as_str() {
        "premium" => Some(aurora_i18n::tr!("world-profiles-premium")),
        "premium_plus" => Some(aurora_i18n::tr!("world-profiles-premium-plus")),
        _ => None,
    }
}

/// The account badge of the Aurora profile's header: a Linden first, then
/// a membership the server names, then Firestorm's other badges. `true`
/// for a Linden (its own color).
pub fn header_badge(p: &AvatarProfile) -> Option<(&'static str, bool)> {
    if p.caption_index == 3 {
        return badge(p).map(|b| (b, true));
    }
    premium(p).or_else(|| badge(p)).map(|b| (b, false))
}

/// "Account" on one line (the Aurora profile's Infos card): a membership the
/// server names stands for the plain resident.
pub fn account_line(p: &AvatarProfile) -> String {
    let text = account_text(p);
    let text = match premium(p) {
        Some(kind) if p.caption_text.is_empty() && p.caption_index == 0 => text.replacen(aurora_i18n::tr!("common-resident"), kind, 1),
        _ => text,
    };
    text.replace('\n', " · ")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Online {
    Yes,
    No,
    Unknown,
}

/// Status line of the profile (processProfileProperties /
/// processOnlineStatus). `friend`: (online in our friend list, they let us
/// see it). A friend who hides it but whom the server says online is
/// "unknown" (FIRE-32184). For other residents Aurora shows the `online`
/// key of the reply when the server sends it (Firestorm only uses it to
/// tell "unknown" apart).
pub fn online_status(p: Option<&AvatarProfile>, own: bool, friend: Option<(bool, bool)>) -> Online {
    if own {
        return Online::Yes;
    }
    let server = p.and_then(|p| p.online);
    match friend {
        Some((online, true)) => {
            if online {
                Online::Yes
            } else {
                Online::No
            }
        }
        Some((_, false)) if server == Some(true) => Online::Unknown,
        Some((_, false)) => Online::No,
        None => match server {
            Some(true) => Online::Yes,
            Some(false) => Online::No,
            None => Online::Unknown,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i64, m: u32, d: u32) -> f64 {
        (days_from_civil(y, m, d) * 86_400) as f64
    }

    #[test]
    fn slt_date_follows_daylight_saving_time() {
        // 2026-10-27 07:30 UTC is 00:30 PDT on the 27th (23:30 PST the day before)
        assert_eq!(
            slt_date(1_793_086_200),
            aurora_i18n::tr!("profile-date", month = "10", day = "27", year = "2026")
        );
        // 2026-11-02 07:30 UTC, after the switch: 23:30 PST on the 1st
        assert_eq!(
            slt_date(1_793_086_200 + 6 * 86_400),
            aurora_i18n::tr!("profile-date", month = "11", day = "01", year = "2026")
        );
    }

    #[test]
    fn age_strings() {
        let now = date(2024, 9, 15) + 3600.0;
        assert_eq!(
            age_from_date("fr", date(2007, 5, 12), now),
            format!(
                "17 ans 4 mois; {} jours",
                days_from_civil(2024, 9, 15) - days_from_civil(2007, 5, 12)
            )
        );
        assert_eq!(age_from_date("fr", date(2023, 9, 15), now), "1 an; 366 jours");
        assert_eq!(age_from_date("fr", date(2024, 7, 20), now), "1 mois; 57 jours");
        // borrowing a month: 20 Aug → 15 Sep is 26 days
        assert_eq!(age_from_date("fr", date(2024, 8, 20), now), "3 semaines; 26 jours");
        assert_eq!(age_from_date("fr", date(2024, 9, 14), now), "1 jour");
        assert_eq!(age_from_date("fr", date(2024, 9, 15), now), "Inscrit aujourd'hui");
        assert_eq!(age_from_date("en", date(2023, 9, 15), now), "1 year; 366 days");
        assert_eq!(age_from_date("en", date(2024, 7, 20), now), "1 month; 57 days");
        assert_eq!(age_from_date("en", date(2024, 8, 20), now), "3 weeks; 26 days");
        assert_eq!(age_from_date("en", date(2024, 9, 14), now), "1 day");
    }

    #[test]
    fn birth_and_hidden_age() {
        let mut p = AvatarProfile {
            born: Some(date(2007, 5, 12)),
            ..Default::default()
        };
        let now = date(2024, 9, 15);
        assert!(birth_text_in("fr", &p, false, now).is_some_and(|t| t.starts_with("12/05/2007\n(17 ans 4 mois")));
        p.hide_age = Some(true);
        assert_eq!(birth_text_in("fr", &p, false, now).as_deref(), Some("12/05"));
        // our own profile always shows the full date
        assert!(birth_text_in("fr", &p, true, now).is_some_and(|t| t.starts_with("12/05/2007")));
        assert_eq!(birth_text_in("en", &p, false, now).as_deref(), Some("05/12"));
        assert!(birth_text_in("en", &p, true, now).is_some_and(|t| t.starts_with("05/12/2007\n(17 years 4 months")));
    }

    #[test]
    fn the_header_birth_line_leaves_the_days_to_the_tooltip() {
        let mut p = AvatarProfile {
            born: Some(date(2019, 4, 2)),
            ..Default::default()
        };
        let now = date(2026, 10, 14);
        assert_eq!(birth_line_in("fr", &p, false, now).as_deref(), Some("02/04/2019 · 7 ans 6 mois"));
        assert_eq!(
            birth_line_in("en", &p, false, now).as_deref(),
            Some("04/02/2019 · 7 years 6 months")
        );
        p.born = Some(date(2026, 9, 20));
        assert_eq!(birth_line_in("fr", &p, false, now).as_deref(), Some("20/09/2026 · 3 semaines"));
        p.born = Some(date(2026, 10, 14));
        assert_eq!(
            birth_line_in("fr", &p, false, now).as_deref(),
            Some("14/10/2026 · Inscrit aujourd'hui")
        );
        // a hidden age: the day and the month alone, except on our profile
        p.hide_age = Some(true);
        assert_eq!(birth_line_in("fr", &p, false, now).as_deref(), Some("14/10"));
        assert!(birth_line_in("fr", &p, true, now).is_some_and(|t| t.starts_with("14/10/2026 · ")));
        assert_eq!(birth_line_in("fr", &AvatarProfile::default(), false, now), None);
    }

    #[test]
    fn premium_only_when_the_server_says_so() {
        let mut p = AvatarProfile {
            flags: flags::IDENTIFIED,
            ..Default::default()
        };
        assert_eq!(header_badge(&p), None);
        assert_eq!(
            account_line(&p),
            format!(
                "{} · {}",
                aurora_i18n::tr!("common-resident"),
                aurora_i18n::tr!("world-profiles-payment-info-on-file")
            )
        );
        p.customer_type = "Premium".into();
        assert_eq!(header_badge(&p), Some((aurora_i18n::tr!("world-profiles-premium"), false)));
        assert!(account_line(&p).starts_with(aurora_i18n::tr!("world-profiles-premium")));
        p.customer_type = "Premium_Plus".into();
        assert_eq!(header_badge(&p), Some((aurora_i18n::tr!("world-profiles-premium-plus"), false)));
        // a lifetime membership keeps Firestorm's badge
        p.customer_type = "secondlifetime_premium".into();
        assert_eq!(header_badge(&p), Some((aurora_i18n::tr!("world-profiles-lifetime-premium"), false)));
        // a Linden first, in its own color
        p.caption_index = 3;
        assert_eq!(
            header_badge(&p),
            Some((aurora_i18n::tr!("world-profiles-linden-lab-employee"), true))
        );
    }

    #[test]
    fn account_and_badges() {
        let mut p = AvatarProfile {
            flags: flags::TRANSACTED,
            born: Some(date(2010, 1, 1)),
            ..Default::default()
        };
        assert_eq!(
            account_text(&p),
            format!(
                "{}\n{}",
                aurora_i18n::tr!("common-resident"),
                aurora_i18n::tr!("world-profiles-payment-info-used")
            )
        );
        assert_eq!(badge(&p), None);
        p.customer_type = "Lifetime".into();
        assert_eq!(badge(&p), Some(aurora_i18n::tr!("world-profiles-lifetime-member")));
        p.born = Some(date(2003, 1, 1));
        assert_eq!(badge(&p), Some(aurora_i18n::tr!("world-profiles-original-beta-tester")));
        p.caption_index = 3;
        assert_eq!(account_text(&p), aurora_i18n::tr!("world-profiles-linden-lab-employee"));
        p.caption_text = "El Jefe!".into();
        assert_eq!(account_text(&p), "El Jefe!");
    }

    #[test]
    fn online() {
        let p = AvatarProfile {
            online: Some(true),
            ..Default::default()
        };
        assert_eq!(online_status(None, true, None), Online::Yes);
        assert_eq!(online_status(Some(&p), false, None), Online::Yes);
        assert_eq!(online_status(None, false, None), Online::Unknown);
        assert_eq!(online_status(Some(&p), false, Some((false, false))), Online::Unknown);
        assert_eq!(online_status(None, false, Some((false, true))), Online::No);
        assert_eq!(online_status(None, false, Some((true, true))), Online::Yes);
    }

    #[test]
    fn local_edits() {
        let id = Uuid::from_u128(1);
        let mut ps = Profiles::default();
        ps.insert(AvatarProfile {
            id,
            notes: Some("a".into()),
            ..Default::default()
        });
        ps.apply_local(id, &aurora_llsd::llsd_map! { "sl_about_text" => "Salut", "allow_publish" => true });
        let p = ps.get(&id).expect("cached");
        assert_eq!(p.sl_about, "Salut");
        assert!(p.allow_publish());
        // a legacy reply keeps the notes we had
        ps.insert(AvatarProfile {
            id,
            legacy: true,
            ..Default::default()
        });
        assert_eq!(ps.get(&id).and_then(|p| p.notes.as_deref()), Some("a"));
    }

    /// A picture chosen in the profile window replaces the one the
    /// pictures are drawn from at once, without waiting for a relog.
    #[test]
    fn a_saved_picture_is_shown_at_once() {
        let id = Uuid::from_u128(1);
        let mut ps = Profiles::default();
        ps.insert(AvatarProfile {
            id,
            sl_image: Uuid::from_u128(10),
            fl_image: Uuid::from_u128(11),
            ..Default::default()
        });
        ps.apply_local(id, &aurora_llsd::llsd_map! { "sl_image_id" => Uuid::from_u128(20) });
        assert_eq!(ps.get(&id).map(|p| p.sl_image), Some(Uuid::from_u128(20)));
        assert_eq!(ps.get(&id).map(|p| p.fl_image), Some(Uuid::from_u128(11)));
        // removed: the default picture
        ps.apply_local(id, &aurora_llsd::llsd_map! { "fl_image_id" => Uuid::nil() });
        assert_eq!(ps.get(&id).map(|p| p.fl_image), Some(Uuid::nil()));
    }

    #[test]
    fn a_new_insignia_reaches_the_profiles_group_lists() {
        let (group, other) = (Uuid::from_u128(5), Uuid::from_u128(6));
        let groups = vec![
            aurora_net::profile::ProfileGroup {
                id: group,
                name: "A".into(),
                insignia: Uuid::from_u128(50),
            },
            aurora_net::profile::ProfileGroup {
                id: other,
                name: "B".into(),
                insignia: Uuid::from_u128(60),
            },
        ];
        let mut ps = Profiles::default();
        for id in [1, 2] {
            ps.insert(AvatarProfile {
                id: Uuid::from_u128(id),
                groups: groups.clone(),
                ..Default::default()
            });
        }
        ps.group_insignia(group, Uuid::from_u128(70));
        for id in [1, 2] {
            let p = ps.get(&Uuid::from_u128(id)).expect("cached");
            assert_eq!(p.groups[0].insignia, Uuid::from_u128(70));
            assert_eq!(p.groups[1].insignia, Uuid::from_u128(60));
        }
    }
}
