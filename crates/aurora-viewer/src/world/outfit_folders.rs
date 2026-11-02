//! Folders of My Outfits that group outfits. The Legacy Viewer accepts
//! plain folders there (can_move_to_my_outfits_as_subfolder and
//! myoutfit_object_subfolder_type, indra/newview/llinventoryfunctions.cpp)
//! and lists the outfits they hold flat (LLOutfitListBase::refreshList and
//! isOutfitFolder, lloutfitslist.cpp; originally LGPL 2.1), leaving the
//! folders to the inventory. Aurora shows them as groups in Appearance and
//! creates, renames, moves and deletes them there (#1002); the inventory
//! keeps the same layout, so the Legacy Viewer still lists every outfit.

use super::appearance::{FT_MY_OUTFITS, FT_OUTFIT, base, system_folder};
use super::inventory::Inventory;
use super::inventory::actions;
use aurora_llsd::llsd_map;
use aurora_net::inventory::operations::{Mutation, Operation};
use uuid::Uuid;

/// Levels of groups under My Outfits: a folder moved there may hold three
/// levels of folders below it (can_move_to_my_outfits_as_subfolder).
pub const MAX_DEPTH: usize = 4;

/// LLFolderType::FT_NONE, the type of a plain folder.
const FT_NONE: i32 = -1;

/// A row of the outfit tree: a group or an outfit, `depth` groups below My
/// Outfits (0 for what My Outfits holds itself).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub id: Uuid,
    pub depth: usize,
    pub group: bool,
    /// The outfits shown in a group, its subgroups' included.
    pub outfits: usize,
}

pub fn my_outfits(inv: &Inventory) -> Option<Uuid> {
    system_folder(inv, FT_MY_OUTFITS)
}

/// An outfit folder of our own, wherever it is.
fn outfit(inv: &Inventory, id: Uuid) -> bool {
    inv.folders.get(&id).is_some_and(|f| f.info.type_default == FT_OUTFIT && !f.library)
}

/// A plain folder of our own that is not an old-style outfit:
/// isOutfitFolder takes a plain folder with more than three items and no
/// subfolder for an outfit made before outfit folders had their own type.
fn plain_group(inv: &Inventory, id: Uuid) -> bool {
    inv.folders
        .get(&id)
        .is_some_and(|f| f.info.type_default == FT_NONE && !f.library && !(f.children.is_empty() && f.items.len() > 3))
}

/// How many groups lead from My Outfits to `id`, `id` included: 0 for My
/// Outfits itself, `None` when `id` is neither My Outfits nor a group in it.
pub fn depth(inv: &Inventory, id: Uuid) -> Option<usize> {
    let my = my_outfits(inv)?;
    let mut depth = 0;
    let mut at = id;
    while at != my {
        if !plain_group(inv, at) || depth > MAX_DEPTH {
            return None;
        }
        depth += 1;
        at = inv.folders.get(&at)?.info.parent;
    }
    Some(depth)
}

/// A group of My Outfits.
pub fn is_group(inv: &Inventory, id: Uuid) -> bool {
    depth(inv, id).is_some_and(|d| d > 0)
}

/// My Outfits or one of its groups: where an outfit or a group can sit.
pub fn container(inv: &Inventory, id: Uuid) -> bool {
    depth(inv, id).is_some()
}

/// An outfit of My Outfits, at its top or in a group, as `tree` finds it.
pub fn in_my_outfits(inv: &Inventory, id: Uuid) -> bool {
    outfit(inv, id)
        && inv
            .folders
            .get(&id)
            .and_then(|f| inv.folders.get(&f.info.parent))
            .is_some_and(|parent| parent.children.contains(&id) && container(inv, parent.info.id))
}

/// The groups and outfits a container holds, each sorted by name.
fn children(inv: &Inventory, id: Uuid, reverse: bool) -> (Vec<(String, Uuid)>, Vec<(String, Uuid)>) {
    let mut groups = Vec::new();
    let mut outfits = Vec::new();
    for child in inv.folders.get(&id).into_iter().flat_map(|f| &f.children) {
        let Some(f) = inv.folders.get(child) else {
            continue;
        };
        if f.info.parent != id {
            continue;
        }
        let named = (f.info.name.to_lowercase(), *child);
        if outfit(inv, *child) {
            outfits.push(named);
        } else if plain_group(inv, *child) {
            groups.push(named);
        }
    }
    for list in [&mut groups, &mut outfits] {
        list.sort();
        if reverse {
            list.reverse();
        }
    }
    (groups, outfits)
}

/// How `tree` orders a folder's groups and outfits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// The list: groups first, like the inventory's folders.
    List,
    /// The gallery: a group's own outfits before its subgroups, so that
    /// its section comes before theirs; My Outfits keeps its groups first.
    Gallery,
}

/// The outfit tree of My Outfits in display order (`Order`), each kind by
/// name (`reverse` for Z to A). A filter (`needle`, lower case) keeps the
/// outfits whose name or one of whose groups' names holds it, and the
/// groups that keep one; without a filter every group shows, empty ones
/// too, so that a new one can be used.
pub fn tree(inv: &Inventory, needle: &str, reverse: bool, order: Order) -> Vec<Entry> {
    let mut entries = Vec::new();
    if let Some(my) = my_outfits(inv) {
        let walk = Walk {
            inv,
            needle,
            reverse,
            order,
        };
        walk.folder(my, 0, needle.is_empty(), &mut entries);
    }
    entries
}

struct Walk<'a> {
    inv: &'a Inventory,
    needle: &'a str,
    reverse: bool,
    order: Order,
}

impl Walk<'_> {
    /// Adds what the container `id` shows; how many outfits that is.
    fn folder(&self, id: Uuid, depth: usize, matched: bool, out: &mut Vec<Entry>) -> usize {
        let (groups, outfits) = children(self.inv, id, self.reverse);
        let outfits_first = self.order == Order::Gallery && depth > 0;
        let mut count = 0;
        if outfits_first {
            count += self.outfits(&outfits, depth, matched, out);
        }
        count += self.groups(&groups, depth, matched, out);
        if !outfits_first {
            count += self.outfits(&outfits, depth, matched, out);
        }
        count
    }

    fn outfits(&self, outfits: &[(String, Uuid)], depth: usize, matched: bool, out: &mut Vec<Entry>) -> usize {
        let mut count = 0;
        for (name, outfit) in outfits {
            if matched || name.contains(self.needle) {
                out.push(Entry {
                    id: *outfit,
                    depth,
                    group: false,
                    outfits: 0,
                });
                count += 1;
            }
        }
        count
    }

    fn groups(&self, groups: &[(String, Uuid)], depth: usize, matched: bool, out: &mut Vec<Entry>) -> usize {
        let mut count = 0;
        for (name, group) in groups {
            if depth >= MAX_DEPTH {
                break;
            }
            let at = out.len();
            out.push(Entry {
                id: *group,
                depth,
                group: true,
                outfits: 0,
            });
            let inner = self.folder(*group, depth + 1, matched || name.contains(self.needle), out);
            if inner == 0 && !self.needle.is_empty() {
                out.truncate(at);
                continue;
            }
            out[at].outfits = inner;
            count += inner;
        }
        count
    }
}

/// Every group and outfit of My Outfits, for Appearance to keep loaded:
/// a group's outfits are known only once the group itself is.
pub fn folders(inv: &Inventory) -> Vec<Uuid> {
    let mut out = Vec::new();
    let Some(my) = my_outfits(inv) else {
        return out;
    };
    let mut stack = vec![(my, 0)];
    while let Some((id, depth)) = stack.pop() {
        for child in inv.folders.get(&id).into_iter().flat_map(|f| &f.children) {
            if outfit(inv, *child) {
                out.push(*child);
            } else if plain_group(inv, *child) && depth < MAX_DEPTH {
                out.push(*child);
                stack.push((*child, depth + 1));
            }
        }
    }
    out
}

/// The levels of groups `id` holds, itself included (0 for an outfit),
/// counted no further than one past `MAX_DEPTH`: a group so deep is never
/// moved, and the walk of a cycle stops there.
fn height(inv: &Inventory, id: Uuid) -> usize {
    fn below(inv: &Inventory, id: Uuid, level: usize) -> usize {
        if level > MAX_DEPTH || !plain_group(inv, id) {
            return 0;
        }
        let deepest = inv
            .folders
            .get(&id)
            .into_iter()
            .flat_map(|f| &f.children)
            .map(|child| below(inv, *child, level + 1))
            .max()
            .unwrap_or(0);
        deepest + 1
    }
    below(inv, id, 0)
}

/// Whether `id` (an outfit or a group) can be moved into the container
/// `to`: never into itself nor below itself, nor deeper than `MAX_DEPTH`.
pub fn can_move(inv: &Inventory, id: Uuid, to: Uuid) -> bool {
    let Some(depth) = depth(inv, to) else {
        return false;
    };
    let movable = in_my_outfits(inv, id) || is_group(inv, id);
    movable && id != to && !actions::under(inv, to, id) && depth + height(inv, id) <= MAX_DEPTH
}

/// Moves outfits and groups into the container `to`; those already there
/// stay. LLViewerInventoryCategory::updateParentOnServer moves them (a
/// `parent_id` patch is sent as MoveInventoryFolder).
pub fn move_into(inv: &Inventory, ids: &[Uuid], to: Uuid) -> Result<Mutation, String> {
    let mut change = Mutation::default();
    for id in actions::roots(inv, ids) {
        if !can_move(inv, id, to) {
            return Err(aurora_i18n::tr!("world-outfit-folders-cannot-move-here").into());
        }
        let Some(from) = inv.folders.get(&id).map(|f| f.info.parent) else {
            return Err(aurora_i18n::tr!("common-item-not-found").into());
        };
        if from == to {
            continue;
        }
        change.operations.push(Operation::Patch {
            id,
            folder: true,
            body: llsd_map! { "parent_id" => to },
        });
        change.refresh.extend([from, to]);
    }
    if change.operations.is_empty() {
        return Err(aurora_i18n::tr!("world-outfit-folders-already-there").into());
    }
    change.refresh.sort();
    change.refresh.dedup();
    Ok(change)
}

/// A new, empty group in the container `parent`.
pub fn create(inv: &Inventory, parent: Uuid, name: &str) -> Result<Mutation, String> {
    match depth(inv, parent) {
        Some(depth) if depth < MAX_DEPTH => actions::create_folder(parent, name),
        Some(_) => Err(aurora_i18n::tr!("world-outfit-folders-too-deep").into()),
        None => Err(aurora_i18n::tr!("world-outfit-folders-cannot-move-here").into()),
    }
}

/// A new group in `parent` and these outfits or groups moved into it, in
/// one mutation: the moves follow the folder's creation and take the id the
/// server gives it.
pub fn create_and_move(inv: &Inventory, parent: Uuid, name: &str, ids: &[Uuid]) -> Result<Mutation, String> {
    let mut change = create(inv, parent, name)?;
    let Some(Operation::Folder { id: new, .. }) = change.operations.first().cloned() else {
        return Err(aurora_i18n::tr!("common-item-not-found").into());
    };
    let level = depth(inv, parent).unwrap_or(MAX_DEPTH) + 1;
    for id in actions::roots(inv, ids) {
        let movable = in_my_outfits(inv, id) || is_group(inv, id);
        if !movable || id == parent || actions::under(inv, parent, id) || level + height(inv, id) > MAX_DEPTH {
            return Err(aurora_i18n::tr!("world-outfit-folders-cannot-move-here").into());
        }
        let Some(from) = inv.folders.get(&id).map(|f| f.info.parent) else {
            return Err(aurora_i18n::tr!("common-item-not-found").into());
        };
        change.operations.push(Operation::Patch {
            id,
            folder: true,
            body: llsd_map! { "parent_id" => new },
        });
        change.refresh.push(from);
    }
    change.refresh.sort();
    change.refresh.dedup();
    Ok(change)
}

pub fn rename(inv: &Inventory, id: Uuid, name: &str) -> Result<Mutation, String> {
    if !is_group(inv, id) {
        return Err(aurora_i18n::tr!("common-item-not-found").into());
    }
    actions::patch(inv, id, llsd_map! { "name" => actions::clean_name(name)? })
}

/// Whether a group holds the outfit being worn, which then cannot go to
/// the Trash (getCanRemoveOutfit keeps the current outfit).
pub fn holds_current(inv: &Inventory, id: Uuid) -> bool {
    base(inv).is_some_and(|current| current != id && actions::under(inv, current, id))
}

/// Moves a group, with the outfits it holds, to the Trash.
pub fn trash(inv: &Inventory, id: Uuid) -> Result<Mutation, String> {
    if !is_group(inv, id) {
        return Err(aurora_i18n::tr!("common-item-not-found").into());
    }
    if holds_current(inv, id) {
        return Err(aurora_i18n::tr!("world-appearance-the-current-outfit-cannot-be-deleted").into());
    }
    let trash = system_folder(inv, 14).ok_or(aurora_i18n::tr!("world-appearance-the-trash-folder-is-unavailable"))?;
    actions::patch(inv, id, llsd_map! { "parent_id" => trash })
}

/// The containers an outfit or a group can be moved to, in tree order:
/// My Outfits (depth 0), then every group.
pub fn targets(inv: &Inventory) -> Vec<(Uuid, usize)> {
    let mut out: Vec<_> = my_outfits(inv).map(|my| (my, 0)).into_iter().collect();
    out.extend(
        tree(inv, "", false, Order::List)
            .into_iter()
            .filter(|e| e.group)
            .map(|e| (e.id, e.depth + 1)),
    );
    out
}

/// The names of the groups from My Outfits down to `id`, joined, for the
/// sections of the gallery ("Beach › Night").
pub fn path(inv: &Inventory, id: Uuid) -> String {
    let mut names = Vec::new();
    let mut at = id;
    while is_group(inv, at) {
        let Some(f) = inv.folders.get(&at) else { break };
        names.push(f.info.name.as_str());
        at = f.info.parent;
    }
    names.reverse();
    names.join(" › ")
}

/// Offline fixture for `AURORA_DEMO_APPEARANCE=folders…`: a "Club" group
/// with two outfits, a "Beach" group with one and a "Night" subgroup with
/// one, and an empty "Spring" group, beside the four demo outfits.
pub fn seed_demo(inv: &mut Inventory, agent: Uuid) {
    use super::inventory::{FetchState, Folder};
    let Some(my) = my_outfits(inv) else {
        return;
    };
    let folder = |id: u128, parent: Uuid, name: &str, kind: i32| Folder {
        info: aurora_net::inventory::InvFolder {
            id: Uuid::from_u128(id),
            parent,
            name: name.into(),
            type_default: kind,
            version: 1,
            ..Default::default()
        },
        children: Vec::new(),
        items: Vec::new(),
        state: FetchState::Fetched,
        library: false,
    };
    let club = Uuid::from_u128(740);
    let beach = Uuid::from_u128(741);
    let night = Uuid::from_u128(742);
    let layout = [
        (740, my, "Club", FT_NONE),
        (741, my, "Beach", FT_NONE),
        (742, beach, "Night", FT_NONE),
        (743, my, "Spring", FT_NONE),
        (744, club, "Club Red", FT_OUTFIT),
        (745, club, "Club Black", FT_OUTFIT),
        (746, beach, "Beach Day", FT_OUTFIT),
        (747, night, "Beach Night", FT_OUTFIT),
    ];
    for (id, parent, name, kind) in layout {
        inv.folders.insert(Uuid::from_u128(id), folder(id, parent, name, kind));
        if let Some(f) = inv.folders.get_mut(&parent) {
            f.children.push(Uuid::from_u128(id));
        }
    }
    // the outfits wear a few of the demo clothes
    let items: Vec<_> = (710..717).map(Uuid::from_u128).filter(|id| inv.items.contains_key(id)).collect();
    for (outfit, take) in [(744, 5), (745, 6), (746, 4), (747, 7)] {
        let links: Vec<_> = items
            .iter()
            .take(take)
            .filter_map(|id| inv.items.get(id))
            .map(super::appearance::item_link)
            .collect();
        super::appearance::demo_replace(inv, agent, Uuid::from_u128(outfit), &links);
    }
}

/// A wardrobe with 320 destination groups for the long Move to folder menu.
pub fn seed_demo_many(inv: &mut Inventory) {
    let Some(my) = my_outfits(inv) else { return };
    let Some(template) = inv.folders.get(&Uuid::from_u128(743)).cloned() else {
        return;
    };
    for index in 0..320u128 {
        let id = Uuid::from_u128(1_485_000 + index);
        let parent = if index.is_multiple_of(4) {
            my
        } else {
            Uuid::from_u128(1_485_000 + index / 4 * 4)
        };
        let mut folder = template.clone();
        folder.info.id = id;
        folder.info.parent = parent;
        folder.info.name = if index.is_multiple_of(4) {
            format!("Wardrobe {:03}", index / 4)
        } else {
            format!("Style {index:03}")
        };
        inv.folders.insert(id, folder);
        if let Some(folder) = inv.folders.get_mut(&parent) {
            folder.children.push(id);
        }
    }
    inv.generation = inv.generation.wrapping_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Inventory {
        let mut inv = Inventory::default();
        super::super::appearance::seed_demo(&mut inv, Uuid::from_u128(1));
        seed_demo(&mut inv, Uuid::from_u128(1));
        inv
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn rows(entries: &[Entry]) -> Vec<(u128, usize, bool, usize)> {
        entries.iter().map(|e| (e.id.as_u128(), e.depth, e.group, e.outfits)).collect()
    }

    #[test]
    fn inventory_outfit_moves_keep_group_rules_and_protection() {
        let inv = demo();
        let empty = std::collections::HashSet::new();
        let my = my_outfits(&inv).expect("My Outfits");
        for (ids, to) in [
            (vec![id(744), id(745)], id(742)),
            (vec![id(747)], my),
            (vec![id(740), id(744)], id(742)),
        ] {
            let change = actions::move_selection(&inv, &ids, to, &empty, &empty).expect("valid wardrobe move");
            assert!(change.move_only());
        }
        for (ids, to) in [
            (vec![id(744)], id(747)),          // an outfit cannot receive another outfit
            (vec![id(741)], id(742)),          // its own descendant
            (vec![id(710)], id(742)),          // a loose item
            (vec![id(744), id(710)], id(742)), // the whole mixed batch is refused
            (vec![id(701)], my),               // Current Outfit is protected
        ] {
            assert!(actions::move_selection(&inv, &ids, to, &empty, &empty).is_err());
        }
        for protected in [id(740), id(744), id(742), id(741), my] {
            assert!(actions::move_selection(&inv, &[id(744)], id(742), &std::collections::HashSet::from([protected]), &empty).is_err());
        }
        let current = base(&inv).expect("current saved outfit");
        let trash = system_folder(&inv, 14).expect("Trash");
        assert!(actions::move_selection(&inv, &[current], trash, &empty, &empty).is_err());
        assert!(actions::move_selection(&inv, &[id(744)], trash, &empty, &empty).is_ok());
        // Four group levels are allowed; an existing subtree cannot make five.
        let mut inv = inv;
        let mut folder = inv.folders[&id(743)].clone();
        folder.info.id = id(748);
        folder.info.parent = id(742);
        inv.folders.get_mut(&id(742)).expect("Night").children.push(id(748));
        inv.folders.insert(id(748), folder.clone());
        folder.info.id = id(749);
        folder.info.parent = id(748);
        inv.folders.get_mut(&id(748)).expect("third level").children.push(id(749));
        inv.folders.insert(id(749), folder);
        assert!(actions::move_selection(&inv, &[id(740)], id(749), &empty, &empty).is_err());
        assert!(actions::move_selection(&inv, &[id(744)], id(749), &empty, &empty).is_ok());
        let mut current_folder = inv.folders[&current].info.clone();
        current_folder.parent = id(740);
        inv.server_updated(vec![current_folder], Vec::new());
        assert!(actions::move_selection(&inv, &[id(740)], trash, &empty, &empty).is_err());
    }

    #[test]
    fn the_tree_lists_groups_first_then_outfits_by_name() {
        let inv = demo();
        let all = rows(&tree(&inv, "", false, Order::List));
        assert_eq!(
            all[..8],
            [
                (741, 0, true, 2),  // Beach
                (742, 1, true, 1),  //   Night
                (747, 2, false, 0), //     Beach Night
                (746, 1, false, 0), //   Beach Day
                (740, 0, true, 2),  // Club
                (745, 1, false, 0), //   Club Black
                (744, 1, false, 0), //   Club Red
                (743, 0, true, 0),  // Spring, empty
            ]
        );
        // the four demo outfits follow, by their names in this language
        let mut loose: Vec<_> = all[8..].iter().map(|r| (r.0, r.1, r.2)).collect();
        loose.sort();
        assert_eq!(loose, (702..706).map(|n| (n, 0, false)).collect::<Vec<_>>());
        let reversed = tree(&inv, "", true, Order::List);
        assert_eq!(reversed[0].id, id(743), "Z to A: Spring first");
        assert!(reversed.last().is_some_and(|e| e.depth == 0 && !e.group), "loose outfits last");
    }

    #[test]
    fn the_gallery_puts_a_groups_outfits_before_its_subgroups() {
        let inv = demo();
        let all = rows(&tree(&inv, "", false, Order::Gallery));
        assert_eq!(
            all[..4],
            [
                (741, 0, true, 2),  // Beach
                (746, 1, false, 0), //   Beach Day
                (742, 1, true, 1),  //   Night
                (747, 2, false, 0), //     Beach Night
            ]
        );
        // My Outfits keeps its groups first, its own outfits last
        assert!(all[8..].iter().all(|r| !r.2 && r.1 == 0));
    }

    #[test]
    fn a_filter_keeps_matching_outfits_and_the_outfits_of_matching_groups() {
        let inv = demo();
        // "night" names a group (its outfit stays) and the outfit itself
        assert_eq!(
            rows(&tree(&inv, "night", false, Order::List)),
            vec![(741, 0, true, 1), (742, 1, true, 1), (747, 2, false, 0)]
        );
        // a group's name keeps all its outfits; empty groups go
        let club = rows(&tree(&inv, "club", false, Order::List));
        assert_eq!(club, vec![(740, 0, true, 2), (745, 1, false, 0), (744, 1, false, 0)]);
        assert!(tree(&inv, "nothing", false, Order::List).is_empty());
    }

    #[test]
    fn old_style_outfits_and_foreign_folders_are_not_groups() {
        let mut inv = demo();
        assert!(is_group(&inv, id(742)) && container(&inv, id(742)));
        assert_eq!(depth(&inv, id(742)), Some(2));
        assert!(container(&inv, my_outfits(&inv).expect("My Outfits")));
        assert!(!is_group(&inv, my_outfits(&inv).expect("My Outfits")));
        assert!(in_my_outfits(&inv, id(747)) && in_my_outfits(&inv, id(704)));
        // a plain folder of more than three items and no subfolder is an
        // old-style outfit (isOutfitFolder), not a group
        inv.folders.get_mut(&id(743)).expect("spring").items = (900..904).map(Uuid::from_u128).collect();
        assert!(!is_group(&inv, id(743)));
        assert!(!tree(&inv, "", false, Order::List).iter().any(|e| e.id == id(743)));
        // the Current Outfit is not under My Outfits
        let cof = super::super::appearance::cof(&inv).expect("current outfit");
        assert!(!container(&inv, cof));
        // a Library outfit is not ours
        inv.folders.get_mut(&id(746)).expect("beach day").library = true;
        assert!(!in_my_outfits(&inv, id(746)));
    }

    #[test]
    fn appearance_keeps_the_groups_and_their_outfits_loaded() {
        let inv = demo();
        let mut loaded = folders(&inv);
        loaded.sort();
        let mut expected: Vec<_> = (702..706).chain(740..748).map(Uuid::from_u128).collect();
        expected.sort();
        assert_eq!(loaded, expected);
    }

    #[test]
    fn moves_stay_within_my_outfits_and_its_depth() {
        let inv = demo();
        let my = my_outfits(&inv).expect("My Outfits");
        // outfits into a group, out of one
        let change = move_into(&inv, &[id(704), id(744)], id(742)).expect("moved");
        assert_eq!(change.operations.len(), 2);
        assert!(change.refresh.contains(&my) && change.refresh.contains(&id(740)) && change.refresh.contains(&id(742)));
        assert!(move_into(&inv, &[id(747)], my).is_ok());
        // already there
        assert!(move_into(&inv, &[id(744)], id(740)).is_err());
        // a group never into itself or below itself
        assert!(!can_move(&inv, id(741), id(741)));
        assert!(!can_move(&inv, id(741), id(742)));
        assert!(can_move(&inv, id(740), id(742)));
        // never outside My Outfits, never the COF
        let cof = super::super::appearance::cof(&inv).expect("current outfit");
        assert!(!can_move(&inv, id(704), cof));
        assert!(!can_move(&inv, cof, my));
        // the move is a parent change, sent as MoveInventoryFolder
        assert!(change.move_only());
    }

    #[test]
    fn groups_go_no_deeper_than_the_legacy_limit() {
        let mut inv = demo();
        let mut parent = id(742);
        for (n, level) in (760..).zip(3..=MAX_DEPTH) {
            assert!(create(&inv, parent, "Deeper").is_ok(), "level {level}");
            let mut f = inv.folders[&id(743)].clone();
            f.info.id = id(n);
            f.info.parent = parent;
            inv.folders.insert(id(n), f);
            if let Some(p) = inv.folders.get_mut(&parent) {
                p.children.push(id(n));
            }
            parent = id(n);
        }
        assert_eq!(depth(&inv, parent), Some(MAX_DEPTH));
        assert!(create(&inv, parent, "Too deep").is_err());
        // Beach holds three levels now: it cannot go below Club
        assert!(!can_move(&inv, id(741), id(740)));
        assert!(create(&inv, super::super::appearance::cof(&inv).expect("cof"), "x").is_err());
    }

    #[test]
    fn a_new_group_can_take_the_selection_at_once() {
        let inv = demo();
        let my = my_outfits(&inv).expect("My Outfits");
        let change = create_and_move(&inv, my, "Work", &[id(703), id(744)]).expect("created");
        let Some(Operation::Folder { id: new, parent, .. }) = change.operations.first() else {
            panic!("the folder comes first: {change:?}");
        };
        assert_eq!(*parent, my);
        assert_eq!(change.operations.len(), 3);
        assert!(
            change.operations[1..]
                .iter()
                .all(|op| matches!(op, Operation::Patch { body, .. } if body["parent_id"].as_uuid() == *new))
        );
        assert!(change.refresh.contains(&my) && change.refresh.contains(&id(740)));
        // a group cannot go into a new folder made below itself
        assert!(create_and_move(&inv, id(742), "Inner", &[id(741)]).is_err());
        // nor anything from outside My Outfits
        let cof = super::super::appearance::cof(&inv).expect("current outfit");
        assert!(create_and_move(&inv, my, "Odd", &[cof]).is_err());
    }

    #[test]
    fn a_group_holding_the_worn_outfit_stays_out_of_the_trash() {
        let mut inv = demo();
        assert!(trash(&inv, id(740)).is_ok());
        assert!(trash(&inv, id(704)).is_err(), "an outfit is not a group");
        // the worn outfit (Leather jacket) moves into Club
        inv.folders.get_mut(&id(705)).expect("worn").info.parent = id(740);
        assert!(holds_current(&inv, id(740)));
        assert!(trash(&inv, id(740)).is_err());
        assert!(rename(&inv, id(740), "  Clubbing ").is_ok());
        assert!(rename(&inv, id(740), "   ").is_err());
        assert!(rename(&inv, id(705), "Outfit").is_err());
    }

    #[test]
    fn targets_and_paths_follow_the_tree() {
        let inv = demo();
        let my = my_outfits(&inv).expect("My Outfits");
        assert_eq!(targets(&inv), vec![(my, 0), (id(741), 1), (id(742), 2), (id(740), 1), (id(743), 1)]);
        assert_eq!(path(&inv, id(742)), "Beach › Night");
        assert_eq!(path(&inv, my), "");
    }
}
