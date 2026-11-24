//! Clean up's demo finder: the demo copies stores hand out to try a thing
//! on (`[Store] Aftermath Shirt - DEMO`), listed for Pre-Trash (never
//! deleted: the user reviews them there, and a run is undone like any).
//!
//! No Legacy Viewer counterpart: it has no demo finder.
//!
//! - A demo is a name holding one of the vocabulary's `demo_words` as a
//!   whole word (`DEMO`, `[Demo]`, `_demo`, `Demo Pack`), never a word
//!   built on it (`Demon`, `Demolition`, `Democracy`).
//! - A folder named as a demo (a delivered demo) goes whole, never opened;
//!   any other folder is looked into, a store's delivery included: a demo
//!   item is never the thing bought.
//! - Links, worn entries and the folders Clean up never reads stay out, as
//!   for the other finders.

use super::{
    Cleanup, CleanupPlan, CleanupRow, FetchState, Inventory, Kind, NEVER_READ, Preferences, VOCABULARY, actions, base_folder, folder_open,
    holds_closed, movable, pre_trash, pre_trash_folders,
};
use std::collections::HashSet;
use uuid::Uuid;

/// A name that says it is a demo: one of the vocabulary's demo words as a
/// whole word. A word joined to others by dots is a store's name, never a
/// demo (`DEMO.RAMA.SALON - Riley Hair`): a demo missed is left alone, a
/// store's every product taken for one would go to Pre-Trash.
pub fn is_demo(name: &str) -> bool {
    name.split(|c: char| !c.is_alphanumeric() && c != '.')
        .map(|w| w.trim_matches('.'))
        .any(|w| VOCABULARY.demo_words.iter().any(|d| w.eq_ignore_ascii_case(d)))
}

/// The demos below `source` (the whole inventory with `cleanup.whole`), of
/// the kinds chosen. Never read, as for [`super::plan_cleanup`]: the Trash,
/// Marketplace Listings, the Library, Favorites, outfits, the Current
/// Outfit, Pre-Trash, the locked default folders, excluded and locked
/// folders, and the sort folder's tree unless `include_sorted`.
pub fn plan_demos(
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
    let mut stack = vec![source];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) || closed(id) {
            continue;
        }
        let Some(folder) = inv.folders.get(&id) else { continue };
        if folder.state != FetchState::Fetched {
            plan.unfetched.push(id);
            continue;
        }
        for item in &folder.items {
            let Some(it) = inv.items.get(item) else { continue };
            if matches!(it.asset_type, 24 | 25)
                || !Kind::of(it.asset_type).is_some_and(|k| cleanup.kinds.contains(&k))
                || !is_demo(&it.name)
                || !movable(inv, *item, protected, &holds_worn)
            {
                continue;
            }
            plan.rows.push(CleanupRow {
                id: *item,
                folder: false,
                from: id,
                name: it.name.clone(),
                kind: Kind::of(it.asset_type),
                created: it.created_at,
                items: 1,
            });
        }
        for sub in &folder.children {
            let Some(f) = inv.folders.get(sub) else { continue };
            if closed(*sub) {
                continue;
            }
            // (a delivered demo goes whole, once all it holds is read and
            // nothing in it is worn or excluded)
            if f.info.type_default == -1 && Some(*sub) != base && is_demo(&f.info.name) {
                let (items, newest) = super::folder_items(inv, *sub);
                if movable(inv, *sub, protected, &holds_worn) && !holds_closed(inv, *sub, &prefs.excluded) {
                    if super::unloaded_below(inv, *sub, &[]).is_empty() {
                        plan.rows.push(CleanupRow {
                            id: *sub,
                            folder: true,
                            from: id,
                            name: f.info.name.clone(),
                            kind: None,
                            created: newest,
                            items,
                        });
                    } else {
                        plan.unfetched.extend(super::unloaded_below(inv, *sub, &[]));
                    }
                }
                continue;
            }
            stack.push(*sub);
        }
    }
    plan.unfetched.sort_unstable();
    plan.unfetched.dedup();
    if source == inv.root {
        plan.locked_items = super::locked_items(inv);
    }
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
