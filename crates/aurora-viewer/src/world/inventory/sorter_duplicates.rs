//! Clean up's duplicate finder: copies of the same item, one kept where it
//! is, the others listed for Pre-Trash (never deleted: the user reviews
//! them there, and a run is undone like any).
//!
//! No Legacy Viewer counterpart: it has no duplicate finder.
//!
//! Two entries are copies when they are the same asset (its id, compared
//! here and never shown, written nor sent), of the same type, by the same
//! creator, with the same rights and the same name. An entry whose asset id
//! the server did not give (nil) is never a copy of anything: two different
//! items never merge on their names alone. Links, worn entries and no-copy
//! items (gacha doubles kept to trade) stay out, the last unless asked.

use super::{
    Cleanup, CleanupPlan, CleanupRow, FetchState, Inventory, Kind, NEVER_READ, Preferences, actions, base_folder, folder_open, movable,
    own_folders, pre_trash, pre_trash_folders,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// PERM_COPY (indra/llinventory/llpermissionsflags.h).
const PERM_COPY: u32 = 0x8000;

/// What makes two entries copies of one item.
#[derive(Hash, PartialEq, Eq)]
struct Key {
    asset: Uuid,
    asset_type: i32,
    inv_type: i32,
    creator: Uuid,
    owner_mask: u32,
    next_owner_mask: u32,
    name: String,
}

/// A name as copies share it: case and runs of spaces aside.
fn same_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The extra copies under `source` (My Inventory for `cleanup.whole`), at
/// any depth, of the kinds chosen: for each item held more than once, every
/// copy but one. The one kept is a copy already filed (in the sort folder
/// or one of the user's own folders), else the oldest. Never read, as for
/// [`super::plan_cleanup`]: the Trash, Marketplace Listings, the Library,
/// Favorites, outfits, the Current Outfit, Pre-Trash, excluded and locked
/// folders, and the sort folder's tree unless `include_sorted`.
pub fn plan_duplicates(
    inv: &Inventory,
    prefs: &Preferences,
    cleanup: &Cleanup,
    source: Uuid,
    protected: &HashSet<Uuid>,
    worn: &HashSet<Uuid>,
) -> CleanupPlan {
    let mut plan = CleanupPlan::default();
    if cleanup.kinds.is_empty() {
        return plan;
    }
    let source = if cleanup.whole { inv.root } else { source };
    let base = base_folder(inv, prefs);
    let own = own_folders(inv, prefs);
    let holds_worn = actions::worn_ancestors(inv, worn);
    let pre_trash_at = pre_trash(inv);
    let waiting_all = pre_trash_folders(inv);
    let closed = |id: Uuid| {
        inv.folders
            .get(&id)
            .is_none_or(|f| f.library || NEVER_READ.contains(&f.info.type_default))
            || Some(id) == pre_trash_at
            || waiting_all.contains(&id)
            || protected.contains(&id)
            || prefs.excludes(inv, id)
            || !folder_open(inv, id)
            || !cleanup.include_sorted && base.is_some_and(|b| id == b || actions::under(inv, id, b))
    };
    // each copy, by item, and whether it is filed already
    let mut copies: HashMap<Key, Vec<(Uuid, Uuid, bool)>> = HashMap::new();
    let mut stack = vec![(source, false)];
    let mut seen = HashSet::new();
    while let Some((id, filed)) = stack.pop() {
        if !seen.insert(id) || closed(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else { continue };
        if folder.state != FetchState::Fetched {
            plan.unfetched.push(id);
            continue;
        }
        let filed = filed || Some(id) == base || own.contains(&id);
        for item in &folder.items {
            let Some(it) = inv.items.get(item) else { continue };
            if it.asset_id.is_nil()
                // (a link is its original's, never a copy of it)
                || matches!(it.asset_type, 24 | 25)
                || !Kind::of(it.asset_type).is_some_and(|k| cleanup.kinds.contains(&k))
                || !cleanup.no_copy_duplicates && it.owner_mask & PERM_COPY == 0
                || !movable(inv, *item, protected, &holds_worn)
            {
                continue;
            }
            copies
                .entry(Key {
                    asset: it.asset_id,
                    asset_type: it.asset_type,
                    inv_type: it.inv_type,
                    creator: it.creator,
                    owner_mask: it.owner_mask,
                    next_owner_mask: it.next_owner_mask,
                    name: same_name(&it.name),
                })
                .or_default()
                .push((*item, id, filed));
        }
        stack.extend(folder.children.iter().map(|c| (*c, filed)));
    }
    for (_, mut group) in copies.into_iter().filter(|(_, g)| g.len() > 1) {
        // the one kept first: filed, then the oldest, then the smallest id
        let created = |id: &Uuid| {
            inv.items
                .get(id)
                .map_or(i64::MAX, |it| if it.created_at > 0 { it.created_at } else { i64::MAX })
        };
        group.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| created(&a.0).cmp(&created(&b.0)))
                .then_with(|| a.0.cmp(&b.0))
        });
        plan.duplicated += 1;
        for (id, from, _) in group.into_iter().skip(1) {
            let Some(it) = inv.items.get(&id) else { continue };
            plan.rows.push(CleanupRow {
                id,
                folder: false,
                from,
                name: it.name.clone(),
                kind: Kind::of(it.asset_type),
                created: it.created_at,
                items: 1,
            });
        }
    }
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    // by kind (the list's groups), then by name, the copies of one together
    plan.rows.sort_by(|a, b| {
        a.group()
            .cmp(&b.group())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.created.cmp(&b.created))
    });
    plan.items = plan.rows.len();
    plan.ends = (1..=plan.rows.len())
        .filter(|&i| i == plan.rows.len() || plan.rows[i].group() != plan.rows[i - 1].group())
        .collect();
    plan.index = plan.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    plan
}
