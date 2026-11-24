//! "Accept in" of the object contents window: the folder the copy of an
//! object's contents goes into. Port of the choice of
//! LLPanelInventoryOfferFolder::getSelectedFolder and
//! LLFloaterOpenObject::moveToInventory (Catznip, indra/newview/
//! llpanelinventoryoffer.cpp and llfloateropenobject.cpp, by Kitty Barnett,
//! originally LGPL 2.1). The new folder, named after the object, is created
//! inside the chosen folder; "[Default]" keeps the usual place.

use crate::world::inventory::{Inventory, actions};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The system folder type of Received Items (FT_INBOX).
const FT_INBOX: i32 = 50;

/// Where the copy goes, as the window's drop-down offers it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    /// The usual place: the inventory root, or Clothing for a copy to wear.
    #[default]
    Default,
    /// The folder the object was rezzed from ("[Object: <folder>]").
    ObjectFolder,
    /// The Received Items folder.
    ReceivedItems,
}

/// The window's "Accept in" choice, kept in the preferences.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AcceptIn {
    /// The checkbox: off, the usual place is used whatever the choice.
    pub on: bool,
    pub choice: Destination,
}

impl AcceptIn {
    /// The destination a copy asks for: the choice when the box is ticked.
    pub fn requested(self) -> Destination {
        if self.on { self.choice } else { Destination::Default }
    }
}

/// The folder the copy goes into, or None for the usual place. A choice
/// that is not a usable folder now (missing, in Trash, in the Library, or
/// the object's folder unknown) falls back to the usual place, as Catznip
/// does: the copy never stops for it.
pub fn parent(choice: Destination, source: Option<Uuid>, inv: &Inventory) -> Option<Uuid> {
    let folder = match choice {
        Destination::Default => return None,
        Destination::ObjectFolder => source?,
        Destination::ReceivedItems => inv.system_folder(FT_INBOX)?,
    };
    usable(inv, folder).then_some(folder)
}

/// The folder an object's Open window names as "[Object: …]": its source
/// folder, when it is a usable destination.
pub fn object_folder(source: Option<Uuid>, inv: &Inventory) -> Option<Uuid> {
    let folder = source?;
    usable(inv, folder).then_some(folder)
}

/// A folder a new folder may be created in: it exists, and it is not in the
/// Library, the Trash or the Current Outfit. Received Items and its folders
/// are allowed, as Catznip allows them (the Legacy Viewer files received
/// items there). Stricter than Catznip on the outfit tree.
pub fn usable(inv: &Inventory, folder: Uuid) -> bool {
    inv.folders.contains_key(&folder)
        && !actions::library(inv, folder)
        && !actions::in_type(inv, folder, 14)
        && !actions::in_type(inv, folder, 46)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// root (1), My Item (2), Received Items (3, type 50), Trash (4, type 14),
    /// a Library folder (5).
    fn inventory() -> Inventory {
        let mut inv = Inventory {
            root: id(1),
            ..Default::default()
        };
        inv.folders.insert(id(1), folder(1, Uuid::nil(), -1, false));
        for (n, parent, kind, library) in [
            (2, id(1), -1, false),
            (3, id(1), 50, false),
            (4, id(1), 14, false),
            (5, id(1), -1, true),
        ] {
            inv.folders.insert(id(n), folder(n, parent, kind, library));
        }
        inv.folders.get_mut(&id(1)).expect("root").children = vec![id(2), id(3), id(4), id(5)];
        inv
    }

    fn folder(n: u128, parent: Uuid, kind: i32, library: bool) -> crate::world::inventory::Folder {
        crate::world::inventory::Folder {
            info: aurora_net::inventory::InvFolder {
                id: id(n),
                parent,
                name: format!("folder {n}"),
                type_default: kind,
                version: 1,
                favorite: false,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: crate::world::inventory::FetchState::Fetched,
            library,
        }
    }

    #[test]
    fn default_keeps_the_usual_place_whatever_else_is_known() {
        let inv = inventory();
        assert_eq!(parent(Destination::Default, Some(id(2)), &inv), None);
        assert_eq!(parent(Destination::Default, None, &inv), None);
    }

    #[test]
    fn the_object_folder_is_the_destination_when_it_is_usable() {
        let inv = inventory();
        assert_eq!(parent(Destination::ObjectFolder, Some(id(2)), &inv), Some(id(2)));
        assert_eq!(object_folder(Some(id(2)), &inv), Some(id(2)));
    }

    #[test]
    fn an_unknown_trashed_or_library_object_folder_falls_back() {
        let inv = inventory();
        for source in [None, Some(id(9)), Some(id(4)), Some(id(5))] {
            assert_eq!(parent(Destination::ObjectFolder, source, &inv), None, "{source:?}");
            assert_eq!(object_folder(source, &inv), None, "{source:?}");
        }
    }

    #[test]
    fn received_items_is_the_inbox_and_falls_back_when_missing() {
        let mut inv = inventory();
        assert_eq!(parent(Destination::ReceivedItems, Some(id(2)), &inv), Some(id(3)));
        inv.folders.remove(&id(3));
        assert_eq!(parent(Destination::ReceivedItems, Some(id(2)), &inv), None);
    }

    #[test]
    fn the_checkbox_off_means_the_usual_place() {
        let on = AcceptIn {
            on: true,
            choice: Destination::ReceivedItems,
        };
        assert_eq!(on.requested(), Destination::ReceivedItems);
        let off = AcceptIn { on: false, ..on };
        assert_eq!(off.requested(), Destination::Default);
        assert_eq!(AcceptIn::default().requested(), Destination::Default);
    }
}
