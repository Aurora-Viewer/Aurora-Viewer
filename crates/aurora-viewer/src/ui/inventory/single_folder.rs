//! "Open in a new window": a whole-inventory window in the Legacy Viewer's
//! single folder mode, on one folder. Port of
//! LLPanelMainInventory::newFolderWindow, onBackFolderClicked,
//! onForwardFolderClicked, onUpFolderClicked and updateNavButtons
//! (indra/newview/llpanelmaininventory.cpp), and of the navigation history
//! of LLInventorySingleFolderPanel (changeFolderRoot, onBackwardFolder,
//! onForwardFolder, isBackwardAvailable, isForwardAvailable,
//! indra/newview/llinventorypanel.cpp), originally LGPL 2.1.
//!
//! The folder shown is the window's `browse`, as in Aurora's explorer: in
//! the Aurora style the explorer draws the window, in the Legacy style
//! `contents` lists the folder flat under the Back, Forward and Up buttons
//! of panel_main_inventory.xml (`nav_buttons`).

use super::*;

/// Where a single folder window has been: its Back and Forward folders.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nav {
    current: Uuid,
    back: Vec<Uuid>,
    forward: Vec<Uuid>,
}

impl Nav {
    pub fn at(folder: Uuid) -> Self {
        Self {
            current: folder,
            ..Self::default()
        }
    }

    /// LLInventorySingleFolderPanel::changeFolderRoot: the folder left goes
    /// on the Back list. Like the Legacy Viewer, Forward is kept.
    pub fn follow(&mut self, folder: Uuid) -> bool {
        if folder == self.current {
            return false;
        }
        if !self.current.is_nil() {
            self.back.push(self.current);
        }
        self.current = folder;
        true
    }

    /// isBackwardAvailable: a folder to go back to, other than this one.
    pub fn can_back(&self) -> bool {
        self.back.last().is_some_and(|last| *last != self.current)
    }

    pub fn can_forward(&self) -> bool {
        self.forward.last().is_some_and(|last| *last != self.current)
    }

    /// onBackwardFolder: the folder to show, this one going on Forward.
    pub fn back(&mut self) -> Option<Uuid> {
        if !self.can_back() {
            return None;
        }
        let folder = self.back.pop()?;
        self.forward.push(self.current);
        self.current = folder;
        Some(folder)
    }

    pub fn forward(&mut self) -> Option<Uuid> {
        if !self.can_forward() {
            return None;
        }
        let folder = self.forward.pop()?;
        self.back.push(self.current);
        self.current = folder;
        Some(folder)
    }
}

/// onUpFolderClicked and updateNavButtons: the parent of the folder shown,
/// none at the top of My Inventory or the Library.
pub(super) fn up(inv: &Inventory, folder: Uuid) -> Option<Uuid> {
    rules::parent(inv, folder).filter(|parent| !parent.is_nil() && inv.folders.contains_key(parent))
}

/// The Legacy-style single folder window: Back, Forward, Up and the folder's
/// name, then the folder window's search and list on that folder. Its folders
/// are gone into rather than opened in place (`InventoryUi::lists_flat`).
#[allow(clippy::too_many_arguments)]
pub(super) fn contents(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    actions: &mut Vec<InvAction>,
) {
    if !inv.folders.contains_key(&st.browse) {
        st.browse = inv.root;
    }
    sync(st);
    let mut go = None;
    ui.horizontal(|ui| {
        let Some(nav) = st.single_folder.as_mut() else { return };
        if crate::ui::widgets::icon_button(ui, p, "arrow-left", aurora_i18n::tr!("inventory-folder-back"), nav.can_back()) {
            go = nav.back();
        }
        if crate::ui::widgets::icon_button(
            ui,
            p,
            "arrow-right",
            aurora_i18n::tr!("inventory-folder-forward"),
            nav.can_forward(),
        ) {
            go = nav.forward();
        }
        let parent = up(inv, st.browse);
        if crate::ui::widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("inventory-folder-up"), parent.is_some()) {
            go = parent;
        }
        ui.label(RichText::new(folder_name(inv, st.browse)).color(p.ink).strong());
    });
    if let Some(folder) = go {
        st.browse = folder;
        // Up is a move like any other (onUpFolderClicked: changeFolderRoot)
        sync(st);
        entered(st);
    }
    let browse = st.browse;
    folder_window_contents(ui, p, icons, inv, browse, st, prefs, facts, actions);
}

/// Bring the history up to the folder shown, whatever moved it (a
/// double-click, Open, a drag held over a folder).
fn sync(st: &mut InventoryUi) {
    let browse = st.browse;
    if st.single_folder.as_mut().is_some_and(|nav| nav.follow(browse)) {
        entered(st);
    }
}

/// A folder just gone into starts at its top with nothing selected.
fn entered(st: &mut InventoryUi) {
    st.selection.clear();
    st.selected = None;
    st.plain_scroll_restore = Some(0.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn back_and_forward_follow_the_legacy_history() {
        let mut nav = Nav::at(id(1));
        assert!(!nav.can_back() && !nav.can_forward());
        assert!(nav.follow(id(2)));
        assert!(nav.follow(id(3)));
        assert!(!nav.follow(id(3)), "the same folder is no move");
        assert_eq!(nav.back(), Some(id(2)));
        assert_eq!(nav.back(), Some(id(1)));
        assert_eq!(nav.back(), None);
        assert_eq!(nav.forward(), Some(id(2)));
        assert!(nav.can_back() && nav.can_forward());
        // changeFolderRoot keeps Forward, as the Legacy Viewer does
        nav.follow(id(9));
        assert_eq!(nav.forward(), Some(id(3)));
        assert_eq!(nav.back(), Some(id(9)));
    }

    #[test]
    fn back_skips_nothing_when_its_folder_is_the_one_shown() {
        let mut nav = Nav::at(id(1));
        nav.follow(id(2));
        nav.follow(id(1));
        // Back holds 1, 2: the last one is not the folder shown
        assert!(nav.can_back());
        let mut same = Nav {
            current: id(5),
            back: vec![id(5)],
            forward: vec![id(5)],
        };
        assert!(!same.can_back() && !same.can_forward());
        assert_eq!(same.back(), None);
        assert_eq!(same.forward(), None);
    }
}
