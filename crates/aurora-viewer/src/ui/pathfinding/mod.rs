//! The pathfinding windows: Region objects (the linksets, with their edit)
//! and Characters. Port of LLFloaterPathfindingObjects,
//! LLFloaterPathfindingLinksets and LLFloaterPathfindingCharacters
//! (indra/newview, floater_pathfinding_linksets.xml and
//! floater_pathfinding_characters.xml, originally LGPL 2.1).
//!
//! The windows keep the Legacy layout (muscle memory) in the Aurora style.
//! The app supplies what the windows cannot know (`Facts`: which actions the
//! in-world selection allows, RLVa) and carries out their `Action`s.
pub mod capsule;
mod edit;
mod list;
pub mod rebake;
#[cfg(test)]
mod tests;
mod view;

pub use edit::{Apply, EditPanel};
pub use list::{Context, List, Messaging};

use aurora_net::pathfinding::Kind;
use aurora_net::pathfinding::linkset::Possible;
use uuid::Uuid;

pub const LINKSETS_WINDOW: &str = "pathfinding_linksets";
pub const CHARACTERS_WINDOW: &str = "pathfinding_characters";

/// What the windows ask of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// requestGetObjects: Refresh list, an opening, a region change.
    Refresh(Kind),
    Take(Vec<Uuid>),
    TakeCopy(Vec<Uuid>),
    Return(Vec<Uuid>),
    Delete(Vec<Uuid>),
    /// teleportToSelectedObject: the object's live position when the viewer
    /// has it, else the one the list reported (region coordinates).
    Teleport {
        id: Uuid,
        listed: [f64; 3],
    },
    /// Apply changes, confirmed.
    Apply(Apply),
    /// The window closed (the app releases its in-world selection).
    Closed(Kind),
}

/// The enable rules of updateStateOnActionControls, worked out by the app
/// from the in-world selection (the Legacy Viewer reads LLSelectMgr's).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// visible_take_object / enable_object_take_copy / enable_object_return
    /// / enable_object_delete over the selected objects the viewer has.
    pub take: bool,
    pub take_copy: bool,
    pub return_: bool,
    pub delete: bool,
    /// RLVa: rlvCanDeleteOrReturn() fails, or fartouch, touchall,
    /// touchworld, touchme, touchthis or interact is on. Greys the actions,
    /// the beacon and the whole edit panel.
    pub rlva_blocked: bool,
    /// RLVa @tplocal: Teleport me to it is greyed.
    pub rlva_no_teleport: bool,
}

/// A question waiting for its answer.
#[derive(Debug, Clone, PartialEq)]
enum Confirm {
    /// PathfindingLinksets_*: the text, then what to send.
    Apply(String, Apply),
    /// PathfindingReturnMultipleItems / PathfindingDeleteMultipleItems.
    Return(Vec<Uuid>),
    Delete(Vec<Uuid>),
}

pub struct PathfindingUi {
    pub linksets: List,
    pub characters: List,
    pub linksets_open: bool,
    pub characters_open: bool,
    pub edit: EditPanel,
    /// The selection the edit fields were last loaded from.
    edit_loaded: Vec<Uuid>,
    /// The uses that selection may take (determinePossibleStates).
    possible: Possible,
    /// Show beacon, per window.
    pub linkset_beacons: bool,
    pub character_beacons: bool,
    /// Show physics capsule (Characters).
    pub capsule: bool,
    confirm: Option<Confirm>,
    /// The navmesh rebake of the agent's region (Rebake region, the icons).
    pub rebake: rebake::Rebake,
    /// PathfindingDirty is open (the rebake icon was clicked).
    pub dirty_question: bool,
}

impl Default for PathfindingUi {
    fn default() -> Self {
        Self {
            linksets: List::new(Kind::Objects),
            characters: List::new(Kind::Characters),
            linksets_open: false,
            characters_open: false,
            edit: EditPanel::default(),
            edit_loaded: Vec::new(),
            possible: Possible::default(),
            linkset_beacons: false,
            character_beacons: false,
            capsule: false,
            confirm: None,
            rebake: rebake::Rebake::default(),
            dirty_question: false,
        }
    }
}

impl PathfindingUi {
    pub fn list(&self, kind: Kind) -> &List {
        match kind {
            Kind::Objects => &self.linksets,
            Kind::Characters => &self.characters,
        }
    }

    pub fn list_mut(&mut self, kind: Kind) -> &mut List {
        match kind {
            Kind::Objects => &mut self.linksets,
            Kind::Characters => &mut self.characters,
        }
    }

    pub fn is_open(&self, kind: Kind) -> bool {
        match kind {
            Kind::Objects => self.linksets_open,
            Kind::Characters => self.characters_open,
        }
    }

    pub fn beacons(&self, kind: Kind) -> bool {
        match kind {
            Kind::Objects => self.linkset_beacons,
            Kind::Characters => self.character_beacons,
        }
    }

    /// showFloaterWithSelectionObjects: open (or bring forward) a window,
    /// the given objects selected once its list arrives. Returns whether a
    /// request is needed (the window was closed).
    pub fn open(&mut self, kind: Kind, select: Vec<Uuid>) -> bool {
        let was_open = self.is_open(kind);
        match kind {
            Kind::Objects => self.linksets_open = true,
            Kind::Characters => self.characters_open = true,
        }
        let list = self.list_mut(kind);
        if was_open && !list.is_pending() {
            list.selection.select_all(select);
            list.mark_dirty();
        } else {
            list.select_when_loaded(select);
        }
        !was_open
    }

    /// The window closed: nothing kept (onClose clears the list, the
    /// selection, the beacons and the capsule).
    pub fn closed(&mut self, kind: Kind) {
        self.list_mut(kind).clear();
        match kind {
            Kind::Objects => {
                self.linkset_beacons = false;
                self.edit = EditPanel::default();
                self.edit_loaded.clear();
                self.possible = Possible::default();
                self.confirm = None;
            }
            Kind::Characters => {
                self.character_beacons = false;
                self.capsule = false;
            }
        }
    }

    /// The selected character whose capsule is shown, if any
    /// (isPhysicsCapsuleEnabled: one row selected and the box ticked).
    pub fn capsule_character(&self) -> Option<Uuid> {
        if !self.characters_open || !self.capsule {
            return None;
        }
        self.characters.selection.single()
    }
}
