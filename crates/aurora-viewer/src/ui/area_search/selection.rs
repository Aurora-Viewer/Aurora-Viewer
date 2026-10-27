//! Selected result rows: the multiple selection of the Legacy Viewer's result
//! list (LLScrollListCtrl with multi_select, floater_fs_area_search.xml).
//! A click selects one row, Ctrl adds or removes one, Shift selects the rows
//! from the last clicked one. A right click keeps a selection of several rows
//! and otherwise selects its row (FSScrollListCtrl::handleRightMouseDown).
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Selection {
    /// In click order; the menu's single-object entries use the only one.
    pub ids: Vec<Uuid>,
    anchor: Option<Uuid>,
}

impl Selection {
    pub fn contains(&self, id: &Uuid) -> bool {
        self.ids.contains(id)
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The object of the entries enabled for one row only ("one").
    pub fn single(&self) -> Option<Uuid> {
        match self.ids.as_slice() {
            [id] => Some(*id),
            _ => None,
        }
    }

    /// A click on row `id`; `order` is the visible rows, top to bottom.
    pub fn click(&mut self, id: Uuid, ctrl: bool, shift: bool, order: impl Fn() -> Vec<Uuid>) {
        if shift && let Some(anchor) = self.anchor {
            let order = order();
            if let (Some(a), Some(b)) = (order.iter().position(|i| *i == anchor), order.iter().position(|i| *i == id)) {
                let range = &order[a.min(b)..=a.max(b)];
                if !ctrl {
                    self.ids.clear();
                }
                for i in range {
                    if !self.ids.contains(i) {
                        self.ids.push(*i);
                    }
                }
                return;
            }
        }
        if ctrl {
            if let Some(at) = self.ids.iter().position(|i| *i == id) {
                self.ids.remove(at);
            } else {
                self.ids.push(id);
            }
        } else {
            self.ids = vec![id];
        }
        self.anchor = Some(id);
    }

    /// A right click on row `id`.
    pub fn context_click(&mut self, id: Uuid) {
        if self.ids.len() < 2 {
            self.ids = vec![id];
            self.anchor = Some(id);
        }
    }

    pub fn select_all(&mut self, ids: Vec<Uuid>) {
        self.anchor = ids.first().copied();
        self.ids = ids;
    }

    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    /// Rows filtered out or gone cannot be acted on.
    pub fn retain(&mut self, visible: &HashSet<Uuid>) {
        self.ids.retain(|id| visible.contains(id));
        if self.anchor.is_some_and(|a| !visible.contains(&a)) {
            self.anchor = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: &[u128]) -> Vec<Uuid> {
        n.iter().map(|i| Uuid::from_u128(*i)).collect()
    }

    #[test]
    fn area_search_selection_follows_click_ctrl_shift_and_right_click() {
        let order = || ids(&[1, 2, 3, 4, 5]);
        let mut s = Selection::default();
        s.click(Uuid::from_u128(2), false, false, order);
        assert_eq!(s.single(), Some(Uuid::from_u128(2)));
        s.click(Uuid::from_u128(4), false, true, order);
        assert_eq!(s.ids, ids(&[2, 3, 4]), "Shift selects from the last clicked row");
        s.click(Uuid::from_u128(3), true, false, order);
        assert_eq!(s.ids, ids(&[2, 4]), "Ctrl takes a selected row out");
        s.click(Uuid::from_u128(5), true, false, order);
        assert_eq!(s.ids, ids(&[2, 4, 5]));
        assert_eq!(s.single(), None);
        s.click(Uuid::from_u128(1), true, true, order);
        assert_eq!(s.ids, ids(&[2, 4, 5, 1, 3]), "Ctrl+Shift adds the range from the anchor");
        s.context_click(Uuid::from_u128(9));
        assert_eq!(s.ids.len(), 5, "a right click keeps several selected rows, even on another row");
        s.click(Uuid::from_u128(4), false, false, order);
        s.context_click(Uuid::from_u128(9));
        assert_eq!(s.ids, ids(&[9]), "a single selection follows the right click");
        s.select_all(order());
        assert_eq!(s.ids, order());
        s.retain(&ids(&[1, 3]).into_iter().collect());
        assert_eq!(s.ids, ids(&[1, 3]));
        s.clear();
        assert!(s.is_empty());
        s.click(Uuid::from_u128(5), false, true, order);
        assert_eq!(s.ids, ids(&[5]), "Shift without an anchor selects one row");
    }
}
