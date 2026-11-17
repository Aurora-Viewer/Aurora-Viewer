//! Empty in small steps: Pre-Trash or the Trash deleted for good in small
//! pieces, for a folder so large that one purge is refused (HTTP 403 on a
//! huge Trash). Aurora's own: the Legacy Viewer's Empty Trash
//! (LLInventoryModel::emptyFolderType, one PurgeDescendents) is left as it
//! is.
//!
//! The pieces go deepest first: a subfolder small enough is deleted whole,
//! a larger one is emptied (its folders, then its items a few at a time)
//! before it is deleted itself, then the folder's own items. A piece the
//! server refuses is split smaller ([`split_piece`]), down to one item.

use super::Inventory;
use uuid::Uuid;

/// The entries (folders and items, at any depth) a folder may hold to be
/// deleted in one request.
pub const PURGE_FOLDER: usize = 200;
/// The items deleted in one piece.
pub const PURGE_ITEMS: usize = 20;

/// One request's worth of a purge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// A folder deleted whole, with all it holds.
    Folder(Uuid),
    /// Items deleted one by one, in one request.
    Items(Vec<Uuid>),
}

impl Piece {
    /// The entries it removes, as the inventory holds them now.
    pub fn entries(&self, inv: &Inventory) -> usize {
        match self {
            Piece::Folder(id) => 1 + entries_below(inv, *id),
            Piece::Items(ids) => ids.len(),
        }
    }

    /// Its first entry: how its request is known again.
    pub fn first(&self) -> Option<Uuid> {
        match self {
            Piece::Folder(id) => Some(*id),
            Piece::Items(ids) => ids.first().copied(),
        }
    }
}

/// The folders and items below `folder`, at any depth.
pub fn entries_below(inv: &Inventory, folder: Uuid) -> usize {
    let mut count = 0;
    let mut stack = vec![folder];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        let Some(f) = inv.folders.get(&id).filter(|_| seen.insert(id)) else {
            continue;
        };
        count += f.items.len() + f.children.len();
        stack.extend(f.children.iter().copied());
    }
    count
}

/// The pieces that empty `folder` (Pre-Trash or the Trash), never the
/// folder itself: its subfolders deepest first, then its own items.
pub fn purge_pieces(inv: &Inventory, folder: Uuid) -> Vec<Piece> {
    let mut out = Vec::new();
    let Some(f) = inv.folders.get(&folder) else { return out };
    for child in &f.children {
        pieces_of(inv, *child, &mut out);
    }
    for chunk in f.items.chunks(PURGE_ITEMS) {
        out.push(Piece::Items(chunk.to_vec()));
    }
    out
}

fn pieces_of(inv: &Inventory, id: Uuid, out: &mut Vec<Piece>) {
    let Some(f) = inv.folders.get(&id) else { return };
    if entries_below(inv, id) <= PURGE_FOLDER {
        out.push(Piece::Folder(id));
        return;
    }
    for child in &f.children {
        pieces_of(inv, *child, out);
    }
    for chunk in f.items.chunks(PURGE_ITEMS) {
        out.push(Piece::Items(chunk.to_vec()));
    }
    // (empty by now)
    out.push(Piece::Folder(id));
}

/// A refused piece in smaller ones: a folder's own folders and its items a
/// few at a time, then the folder; items in halves. None when it cannot be
/// smaller (one item, an empty folder): it is counted as failed.
pub fn split_piece(inv: &Inventory, piece: &Piece) -> Option<Vec<Piece>> {
    match piece {
        Piece::Folder(id) => {
            let f = inv.folders.get(id)?;
            if f.children.is_empty() && f.items.is_empty() {
                return None;
            }
            let mut out: Vec<Piece> = f.children.iter().map(|c| Piece::Folder(*c)).collect();
            out.extend(f.items.chunks(5).map(|c| Piece::Items(c.to_vec())));
            out.push(Piece::Folder(*id));
            Some(out)
        }
        Piece::Items(ids) if ids.len() > 1 => {
            let (a, b) = ids.split_at(ids.len() / 2);
            Some(vec![Piece::Items(a.to_vec()), Piece::Items(b.to_vec())])
        }
        Piece::Items(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::inventory::{FetchState, Folder};
    use aurora_net::inventory::{InvFolder, InvItem};

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn add_folder(inv: &mut Inventory, at: u128, parent: u128) {
        inv.folders.insert(
            id(at),
            Folder {
                info: InvFolder {
                    id: id(at),
                    parent: id(parent),
                    type_default: if at == 1 { 14 } else { -1 },
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: FetchState::Fetched,
                library: false,
            },
        );
        if let Some(p) = inv.folders.get_mut(&id(parent)) {
            p.children.push(id(at));
        }
    }

    fn add_items(inv: &mut Inventory, parent: u128, from: u128, count: u128) {
        inv.add_items(
            (from..from + count)
                .map(|n| InvItem {
                    id: id(n),
                    parent: id(parent),
                    name: format!("Item {n}"),
                    asset_type: 6,
                    inv_type: 6,
                    ..Default::default()
                })
                .collect(),
        );
    }

    /// A huge Trash goes deepest first: small folders whole, a large one
    /// emptied before it goes, then the loose items a few at a time; a
    /// refused piece is split down to one item.
    #[test]
    fn a_huge_trash_is_emptied_in_small_pieces_deepest_first() {
        let mut inv = Inventory::default();
        add_folder(&mut inv, 1, 0);
        // a small folder, a large one holding a small one, loose items
        add_folder(&mut inv, 10, 1);
        add_items(&mut inv, 10, 1_000, 5);
        add_folder(&mut inv, 20, 1);
        add_folder(&mut inv, 21, 20);
        add_items(&mut inv, 21, 2_000, 10);
        add_items(&mut inv, 20, 3_000, 250);
        add_items(&mut inv, 1, 4_000, 45);
        let pieces = purge_pieces(&inv, id(1));
        assert_eq!(pieces[0], Piece::Folder(id(10)), "a small folder whole");
        assert_eq!(pieces[1], Piece::Folder(id(21)), "the large folder's small one first");
        let big = pieces
            .iter()
            .position(|p| *p == Piece::Folder(id(20)))
            .expect("the large folder, once empty");
        assert!(
            pieces[2..big]
                .iter()
                .all(|p| matches!(p, Piece::Items(v) if v.len() <= PURGE_ITEMS)),
            "its items a few at a time before it"
        );
        let loose: usize = pieces[big + 1..]
            .iter()
            .map(|p| match p {
                Piece::Items(v) => v.len(),
                Piece::Folder(_) => 0,
            })
            .sum();
        assert_eq!(loose, 45, "then the loose items");
        assert!(!pieces.contains(&Piece::Folder(id(1))), "never the Trash itself");
        // refused: smaller, down to one item
        let parts = split_piece(&inv, &Piece::Folder(id(10))).expect("splits");
        assert_eq!(parts.last(), Some(&Piece::Folder(id(10))));
        let halves = split_piece(&inv, &Piece::Items(vec![id(1), id(2), id(3)])).expect("halves");
        assert_eq!(halves.len(), 2);
        assert!(split_piece(&inv, &Piece::Items(vec![id(1)])).is_none());
    }
}
