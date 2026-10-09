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
    /// Pick details (PickInfoReply), by pick id.
    pub picks: HashMap<Uuid, PickInfo>,
    /// Classifieds of each avatar (AvatarClassifiedReply).
    pub classified_lists: HashMap<Uuid, Vec<(Uuid, String)>>,
    pub classifieds: HashMap<Uuid, ClassifiedInfo>,
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
        v
    }

    pub fn insert(&mut self, p: AvatarProfile) {
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
}

/// Classified ad texts (LLPanelProfileClassified::processProperties).
pub fn classified_maturity(flags: u8) -> &'static str {
    use aurora_net::profile::classified_flags::{MATURE, QUERY_INC_MATURE};
    if flags & (MATURE | QUERY_INC_MATURE) != 0 {
        "Modéré"
    } else {
        "Contenu Général"
    }
}

/// "mm/dd/yyyy" in SLT (date_fmt of panel_profile_classified.xml).
pub fn slt_date(secs: u32) -> String {
    let secs = secs as i64;
    // SLT is US Pacific time; DST approximated by day of year like the top bar
    let year_day = (secs / 86_400).rem_euclid(365);
    let off = if (68..=307).contains(&year_day) { -7 } else { -8 };
    let (y, m, d) = civil_from_days((secs + off * 3600).div_euclid(86_400));
    format!("{m:02}/{d:02}/{y}")
}

/// French plural of LLTrans::getCountString ("fr": 0 and 1 are singular).
fn count(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 0 || n == 1 { one } else { many })
}

fn days_in_month(y: i64, m: u32) -> i64 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)
}

fn split(secs: f64) -> (i64, u32, u32) {
    civil_from_days((secs / 86_400.0).floor() as i64)
}

/// LLDateUtil::ageFromDate with Firestorm's total days ("TotalDaysOld"):
/// "17 ans 4 mois; 6350 jours", "3 semaines; 23 jours", "5 jours".
pub fn age_from_date(born: f64, now: f64) -> String {
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
    let total = ((now - born) / 86_400.0) as i64;
    let total = format!("; {}", count(total, "jour", "jours"));
    if months > 0 || years > 0 {
        let y = count(years, "an", "ans");
        let m = count(months, "mois", "mois");
        return match (years > 0, months > 0) {
            (true, true) => format!("{y} {m}{total}"),
            (true, false) => format!("{y}{total}"),
            _ => format!("{m}{total}"),
        };
    }
    let weeks = days / 7;
    if weeks > 0 {
        return format!("{}{total}", count(weeks, "semaine", "semaines"));
    }
    if days % 7 > 0 {
        return count(days % 7, "jour", "jours");
    }
    "Inscrit aujourd'hui".to_owned()
}

/// "Date de naissance" (fillAgeData): "dd/mm/yyyy\n(age)", or only "dd/mm"
/// when someone else hides their age. Firestorm's French skin has no
/// AvatarBirthDateFormatFull and falls back to the US month/day order;
/// Aurora keeps the French AvatarBirthDateFormat (day first).
pub fn birth_text(p: &AvatarProfile, own: bool, now: f64) -> Option<String> {
    let born = p.born?;
    let (y, m, d) = split(born);
    if !own && p.hide_age == Some(true) {
        return Some(format!("{d:02}/{m:02}"));
    }
    Some(format!("{d:02}/{m:02}/{y}\n({})", age_from_date(born, now)))
}

/// "Compte" (CaptionTextAcctInfo): account type, then the payment info.
pub fn account_text(p: &AvatarProfile) -> String {
    if !p.caption_text.is_empty() {
        // special accounts (M Linden) show their caption, without payment info
        return p.caption_text.clone();
    }
    let kind = ["Résident", "Essai", "Membre originaire", "Employé(e) de Linden Lab"][p.caption_index.min(3) as usize];
    if p.caption_index >= 3 {
        return kind.to_owned();
    }
    let payment = if p.flags & flags::TRANSACTED != 0 {
        "Infos de paiement utilisées"
    } else if p.flags & flags::IDENTIFIED != 0 {
        "Infos de paiement enregistrées"
    } else {
        "Aucune info de paiement enregistrée"
    };
    format!("{kind}\n{payment}")
}

/// Account badge next to "Compte" (LLPanelProfileSecondLife::fillAccountStatus):
/// the first that applies.
pub fn badge(p: &AvatarProfile) -> Option<&'static str> {
    // the original beta testers registered before 2003-06-23
    let beta_end = (days_from_civil(2003, 6, 23) * 86_400) as f64;
    if p.caption_index == 3 {
        return Some("Employé(e) de Linden Lab");
    }
    if p.born.is_some_and(|b| b < beta_end) {
        return Some("Bêta-testeur originel");
    }
    match p.customer_type.to_lowercase().as_str() {
        "beta_lifetime" => Some("Membre Beta à vie"),
        "lifetime" => Some("Membre à vie"),
        "secondlifetime_premium" => Some("Premium à vie"),
        "secondlifetime_premium_plus" => Some("Premium Plus à vie"),
        _ => None,
    }
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
    fn age_strings() {
        let now = date(2024, 9, 15) + 3600.0;
        assert_eq!(
            age_from_date(date(2007, 5, 12), now),
            format!(
                "17 ans 4 mois; {} jours",
                days_from_civil(2024, 9, 15) - days_from_civil(2007, 5, 12)
            )
        );
        assert_eq!(age_from_date(date(2023, 9, 15), now), "1 an; 366 jours");
        assert_eq!(age_from_date(date(2024, 7, 20), now), "1 mois; 57 jours");
        // borrowing a month: 20 Aug → 15 Sep is 26 days
        assert_eq!(age_from_date(date(2024, 8, 20), now), "3 semaines; 26 jours");
        assert_eq!(age_from_date(date(2024, 9, 14), now), "1 jour");
        assert_eq!(age_from_date(date(2024, 9, 15), now), "Inscrit aujourd'hui");
    }

    #[test]
    fn birth_and_hidden_age() {
        let mut p = AvatarProfile {
            born: Some(date(2007, 5, 12)),
            ..Default::default()
        };
        let now = date(2024, 9, 15);
        assert!(birth_text(&p, false, now).is_some_and(|t| t.starts_with("12/05/2007\n(17 ans 4 mois")));
        p.hide_age = Some(true);
        assert_eq!(birth_text(&p, false, now).as_deref(), Some("12/05"));
        // our own profile always shows the full date
        assert!(birth_text(&p, true, now).is_some_and(|t| t.starts_with("12/05/2007")));
    }

    #[test]
    fn account_and_badges() {
        let mut p = AvatarProfile {
            flags: flags::TRANSACTED,
            born: Some(date(2010, 1, 1)),
            ..Default::default()
        };
        assert_eq!(account_text(&p), "Résident\nInfos de paiement utilisées");
        assert_eq!(badge(&p), None);
        p.customer_type = "Lifetime".into();
        assert_eq!(badge(&p), Some("Membre à vie"));
        p.born = Some(date(2003, 1, 1));
        assert_eq!(badge(&p), Some("Bêta-testeur originel"));
        p.caption_index = 3;
        assert_eq!(account_text(&p), "Employé(e) de Linden Lab");
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
}
