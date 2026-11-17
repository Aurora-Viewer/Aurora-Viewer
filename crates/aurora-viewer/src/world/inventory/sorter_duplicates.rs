//! Clean up's duplicate finder: real doubles, one kept where it is, the
//! others listed for Pre-Trash (never deleted: the user reviews them there,
//! and a run is undone like any).
//!
//! No Legacy Viewer counterpart: it has no duplicate finder.
//!
//! Conservative by design, since a store puts the same notecard, landmark,
//! logo texture or script in every one of its deliveries, and a fatpack the
//! same HUD in each colour's folder:
//! - a product's folder (a delivery, a fatpack) is never opened: nothing is
//!   pulled out of it. It is a double only of another whole delivery that
//!   holds the very same items under the same name, and then goes whole;
//! - loose items count only as objects, clothing, body parts, gestures and
//!   animations (the things a user recognises as "the same thing twice"),
//!   of the kinds chosen; landmarks and notecards only behind their own
//!   switch; textures, scripts, sounds, materials and calling cards never;
//! - two entries are copies when they are the same asset (its id, compared
//!   here and never shown, written nor sent), of the same type, by the same
//!   creator, with the same rights and the same name. An entry whose asset
//!   id the server did not give (nil) is never a copy of anything: two
//!   different items never merge on their names alone;
//! - links, worn entries, no-copy items (gacha doubles kept to trade,
//!   unless asked) and the folders Clean up never reads stay out.

use super::{
    Cleanup, CleanupPlan, CleanupRow, FetchState, Inventory, Kind, NEVER_READ, Preferences, actions, base_folder, folder_open,
    holds_closed, movable, own_folders, pre_trash, pre_trash_folders, product_folder_in, sorter_folder_names,
};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// PERM_COPY (indra/llinventory/llpermissionsflags.h).
const PERM_COPY: u32 = 0x8000;

/// The kinds a loose double is looked for in; landmarks and notecards
/// only when asked (a store's boilerplate, the same in every delivery).
const DOUBLES: [Kind; 5] = [Kind::Objects, Kind::Clothing, Kind::BodyParts, Kind::Gestures, Kind::Animations];

/// What makes two items copies of one.
#[derive(Hash, PartialEq, Eq, Clone, PartialOrd, Ord)]
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

fn key_of(it: &aurora_net::inventory::InvItem) -> Key {
    Key {
        asset: it.asset_id,
        asset_type: it.asset_type,
        inv_type: it.inv_type,
        creator: it.creator,
        owner_mask: it.owner_mask,
        next_owner_mask: it.next_owner_mask,
        name: same_name(&it.name),
    }
}

/// A whole delivery's mark: its name and every item it holds at any depth,
/// or none when one of them could not be told apart (an asset id not
/// given), or it holds nothing.
fn delivery_mark(inv: &Inventory, folder: Uuid) -> Option<(String, Vec<Key>)> {
    let mut keys = Vec::new();
    let mut stack = vec![folder];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let f = inv.folders.get(&id)?;
        if f.state != FetchState::Fetched {
            return None;
        }
        for item in &f.items {
            let it = inv.items.get(item)?;
            if it.asset_id.is_nil() && !matches!(it.asset_type, 24 | 25) {
                return None;
            }
            keys.push(key_of(it));
        }
        stack.extend(f.children.iter().copied());
    }
    if keys.is_empty() {
        return None;
    }
    keys.sort();
    Some((same_name(&inv.folders.get(&folder)?.info.name), keys))
}

/// A delivery the user's choices let in: no no-copy item unless asked,
/// and at least one thing of a kind chosen.
fn delivery_wanted(inv: &Inventory, folder: Uuid, cleanup: &Cleanup) -> bool {
    let mut wanted = false;
    let mut stack = vec![folder];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        let Some(f) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
            continue;
        };
        for it in f.items.iter().filter_map(|i| inv.items.get(i)) {
            if !cleanup.no_copy_duplicates && it.owner_mask & PERM_COPY == 0 && !matches!(it.asset_type, 24 | 25) {
                return false;
            }
            wanted |= Kind::of(it.asset_type).is_some_and(|k| cleanup.kinds.contains(&k));
        }
        stack.extend(f.children.iter().copied());
    }
    wanted
}

/// The doubles under `source` (My Inventory for `cleanup.whole`): for each
/// thing held more than once, every copy but one. The one kept is a copy
/// already filed (in the sort folder or one of the user's own folders),
/// else the oldest. Never read, as for [`super::plan_cleanup`]: the Trash,
/// Marketplace Listings, the Library, Favorites, outfits, the Current
/// Outfit, Pre-Trash, the locked default folders, excluded and locked
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
    let reserved = sorter_folder_names(prefs);
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
    let looked_for: Vec<Kind> = DOUBLES
        .iter()
        .chain(cleanup.landmark_duplicates.then_some(&Kind::Landmarks))
        .chain(cleanup.notecard_duplicates.then_some(&Kind::Notecards))
        .copied()
        .filter(|k| cleanup.kinds.contains(k))
        .collect();
    // each copy, by item or by whole delivery, and whether it is filed
    let mut items: HashMap<Key, Vec<(Uuid, Uuid, bool)>> = HashMap::new();
    let mut deliveries: HashMap<(String, Vec<Key>), Vec<(Uuid, Uuid, bool)>> = HashMap::new();
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
                || !Kind::of(it.asset_type).is_some_and(|k| looked_for.contains(&k))
                || !cleanup.no_copy_duplicates && it.owner_mask & PERM_COPY == 0
                || !movable(inv, *item, protected, &holds_worn)
            {
                continue;
            }
            items.entry(key_of(it)).or_default().push((*item, id, filed));
        }
        for sub in &folder.children {
            let Some(f) = inv.folders.get(sub) else { continue };
            if closed(*sub) {
                continue;
            }
            let system = f.info.type_default != -1 || Some(*sub) == base;
            let made = reserved.contains(&f.info.name.trim().to_lowercase());
            // a product's folder is never opened: a double only whole
            if !system && !made && !own.opens(sub) && product_folder_in(inv, *sub, &reserved) {
                if movable(inv, *sub, protected, &holds_worn)
                    && !holds_closed(inv, *sub, &prefs.excluded)
                    && let Some(mark) = delivery_mark(inv, *sub)
                    // (the same switches as loose items: a no-copy item
                    // inside keeps it out unless asked, and one of its
                    // things must be of a kind chosen)
                    && delivery_wanted(inv, *sub, cleanup)
                {
                    deliveries.entry(mark).or_default().push((*sub, id, filed));
                }
                continue;
            }
            stack.push((*sub, filed));
        }
    }
    let created = |id: &Uuid| {
        inv.items
            .get(id)
            .map_or(i64::MAX, |it| if it.created_at > 0 { it.created_at } else { i64::MAX })
    };
    let newest_in = |folder: &Uuid| {
        let (_, newest) = super::folder_items(inv, *folder);
        if newest > 0 { newest } else { i64::MAX }
    };
    // the one kept first: filed, then the oldest, then the smallest id
    for (_, mut group) in items.into_iter().filter(|(_, g)| g.len() > 1) {
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
    for ((_, keys), mut group) in deliveries.into_iter().filter(|(_, g)| g.len() > 1) {
        group.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then_with(|| newest_in(&a.0).cmp(&newest_in(&b.0)))
                .then_with(|| a.0.cmp(&b.0))
        });
        plan.duplicated += 1;
        plan.whole_deliveries += group.len() - 1;
        for (id, from, _) in group.into_iter().skip(1) {
            let Some(f) = inv.folders.get(&id) else { continue };
            plan.rows.push(CleanupRow {
                id,
                folder: true,
                from,
                name: f.info.name.clone(),
                kind: None,
                created: super::folder_items(inv, id).1,
                items: keys.len(),
            });
        }
    }
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    if source == inv.root {
        plan.locked_items = super::locked_items(inv);
    }
    // by group (whole deliveries first, then each kind), then by name, the
    // copies of one together
    plan.rows.sort_by(|a, b| {
        a.group()
            .cmp(&b.group())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.created.cmp(&b.created))
    });
    plan.items = plan.rows.iter().map(|r| r.items).sum();
    plan.ends = (1..=plan.rows.len())
        .filter(|&i| i == plan.rows.len() || plan.rows[i].group() != plan.rows[i - 1].group())
        .collect();
    plan.index = plan.rows.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
    plan
}
