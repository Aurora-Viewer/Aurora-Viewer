//! The explorer's Group by (Windows Explorer's, Dolphin's "Show in Groups"):
//! the Details list cut into named groups, by type, by date or by creator,
//! each opening and closing on a click of its header. Aurora's: the Legacy
//! Viewer's inventory has no groups. What group an entry falls in, and the
//! groups' order, are here; `rows::PlainTree` lists them.

use super::view::ViewCache;
use super::*;
use serde::{Deserialize, Serialize};

/// What the explorer's list is grouped by.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    #[default]
    None,
    Kind,
    Date,
    Creator,
}

/// An entry's group: its place among the groups, then its name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Group {
    pub rank: u32,
    pub label: String,
}

impl Group {
    fn new(rank: u32, label: impl Into<String>) -> Self {
        Self { rank, label: label.into() }
    }
}

/// The date groups, newest first (Windows Explorer's, shortened): today,
/// yesterday, the last 7 days, the last 31, this year, earlier, no date.
fn date_group(seconds: i64, now: i64) -> Group {
    if seconds <= 0 {
        return Group::new(6, aurora_i18n::tr!("inventory-group-no-date"));
    }
    let days = now.div_euclid(86400) - seconds.div_euclid(86400);
    let (this_year, _, _) = aurora_net::profile::civil_from_days(now.div_euclid(86400));
    let (year, _, _) = aurora_net::profile::civil_from_days(seconds.div_euclid(86400));
    match days {
        ..=0 => Group::new(0, aurora_i18n::tr!("inventory-group-today")),
        1 => Group::new(1, aurora_i18n::tr!("inventory-group-yesterday")),
        2..=6 => Group::new(2, aurora_i18n::tr!("inventory-group-this-week")),
        7..=30 => Group::new(3, aurora_i18n::tr!("inventory-group-this-month")),
        _ if year == this_year => Group::new(4, aurora_i18n::tr!("inventory-group-this-year")),
        _ => Group::new(5, aurora_i18n::tr!("inventory-group-earlier")),
    }
}

/// The group of `id` (a folder or an item of the folder shown).
pub fn of(inv: &Inventory, view: &ViewCache, id: Uuid, by: GroupBy, now: i64) -> Group {
    let folders = || Group::new(0, aurora_i18n::tr!("inventory-group-folders"));
    let item = inv.items.get(&id);
    match by {
        GroupBy::None => Group::new(0, ""),
        GroupBy::Kind => match item {
            // the type's first word: "Clothing" of "Clothing · Skirt"
            Some(_) => {
                let kind = super::aurora::rows::kind_name(inv, id);
                Group::new(1, kind.split(" · ").next().unwrap_or_default().to_owned())
            }
            None => folders(),
        },
        GroupBy::Date => date_group(item.map_or_else(|| view.folder_date(id), |it| it.created_at), now),
        GroupBy::Creator => match item {
            Some(it) => match view.creator_name(inv, it) {
                Some(name) => Group::new(1, name),
                None => Group::new(2, aurora_i18n::tr!("inventory-group-unknown-creator")),
            },
            None => folders(),
        },
    }
}

/// `entries` cut into their groups, in the groups' order, each keeping the
/// list's order inside it.
pub fn split(inv: &Inventory, view: &ViewCache, entries: &[Uuid], by: GroupBy, now: i64) -> Vec<(Group, Vec<Uuid>)> {
    let mut groups: Vec<(Group, Vec<Uuid>)> = Vec::new();
    for id in entries {
        let group = of(inv, view, *id, by, now);
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, members)) => members.push(*id),
            None => groups.push((group, vec![*id])),
        }
    }
    groups.sort_by(|(a, _), (b, _)| {
        a.rank
            .cmp(&b.rank)
            .then_with(|| crate::natural_order::cmp_folded(&a.label.to_lowercase(), &b.label.to_lowercase()))
    });
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_fall_in_the_explorers_groups_newest_first() {
        let now = 1_763_500_000; // 2025-11-18
        let day = 86400;
        let rank = |seconds| date_group(seconds, now).rank;
        assert_eq!(rank(now - 60), 0, "today");
        assert_eq!(rank(now - day), 1, "yesterday");
        assert_eq!(rank(now - 3 * day), 2, "this week");
        assert_eq!(rank(now - 20 * day), 3, "this month");
        assert_eq!(rank(now - 200 * day), 4, "this year");
        assert_eq!(rank(now - 400 * day), 5, "earlier");
        assert_eq!(rank(0), 6, "no date, last");
        assert_eq!(rank(now + day), 0, "a clock ahead counts as today");
    }
}
