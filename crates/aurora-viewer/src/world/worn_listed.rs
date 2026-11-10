//! What an avatar wears against what the simulator lists (issue #2747:
//! avatars that stay in their system body, as if nothing they wear came).
//!
//! The AvatarAppearance message lists the worn objects by id
//! (AttachmentBlock, kept as LLVOAvatar::mSimAttachments,
//! indra/newview/llvoavatar.cpp, originally LGPL 2.1). The Legacy Viewer only
//! compares the counts, to know when to stop waiting; here the Dev Toolbox's
//! J2 also names the ones that never came, at any time, so that a tester's
//! paste tells whether the simulator never listed them, listed them without
//! sending them, or sent them under another key. Diagnostic only: nothing
//! here changes what is drawn or asked for.

use super::World;
use uuid::Uuid;

/// One avatar's worn objects against the simulator's list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WornVsListed {
    /// An appearance came for the avatar (without one there is no list).
    pub appearance: bool,
    /// Ids the simulator lists.
    pub listed: usize,
    /// Worn objects in the scene (roots hanging from the avatar).
    pub arrived: usize,
    /// Of them, those the list names.
    pub arrived_listed: usize,
    /// Listed ones not in the scene: object id and attachment point.
    pub missing: Vec<(Uuid, u8)>,
}

/// Compare a list with the worn roots, each given as its object id and the
/// inventory item it was worn from (AttachItemID, when sent). A listed id
/// matches either: the list names the objects, the item is a fallback in
/// case a simulator names the items instead.
pub fn compare(appearance: bool, listed: &[(Uuid, u8)], worn: &[(Uuid, Option<Uuid>)]) -> WornVsListed {
    let named = |id: &Uuid| worn.iter().any(|(object, item)| object == id || item.as_ref() == Some(id));
    let missing: Vec<(Uuid, u8)> = listed.iter().filter(|(id, _)| !named(id)).copied().collect();
    let arrived_listed = worn
        .iter()
        .filter(|(object, item)| listed.iter().any(|(id, _)| id == object || item.as_ref() == Some(id)))
        .count();
    WornVsListed {
        appearance,
        listed: listed.len(),
        arrived: worn.len(),
        arrived_listed,
        missing,
    }
}

impl WornVsListed {
    /// The J2 line (object ids and point numbers only: no names, no assets).
    pub fn line(&self) -> String {
        if !self.appearance {
            return format!("worn objects: no appearance received yet, {} arrived", self.arrived);
        }
        let mut out = format!(
            "worn objects: {} listed by the simulator, {} arrived ({} of the listed, {} not listed)",
            self.listed,
            self.arrived,
            self.arrived_listed,
            self.arrived - self.arrived_listed
        );
        if self.missing.is_empty() {
            out.push_str(", none missing");
        } else {
            let ids: Vec<String> = self.missing.iter().map(|(id, point)| format!("{id} on point {point}")).collect();
            out.push_str(&format!(", {} missing: {}", self.missing.len(), ids.join(", ")));
        }
        out
    }
}

impl World {
    /// What the simulator lists for the avatar in its last appearance
    /// message, kept or not (LLVOAvatar::mSimAttachments); empty when it
    /// listed none.
    pub fn listed_attachments(&self, avatar: &Uuid) -> &[(Uuid, u8)] {
        self.listed_worn.get(avatar).map_or(&[], Vec::as_slice)
    }

    /// The avatar at `idx`, its worn objects against the simulator's list.
    pub fn worn_vs_listed(&self, idx: usize) -> Option<WornVsListed> {
        let av = self.objects.get(idx).filter(|o| o.is_avatar())?;
        let worn: Vec<(Uuid, Option<Uuid>)> = self
            .objects
            .children_of(&av.key)
            .iter()
            .filter_map(|&i| self.objects.get(i))
            .filter(|o| !o.is_avatar())
            .map(|o| (o.full_id, o.attachment_item_id()))
            .collect();
        let appearance = self.listed_worn.contains_key(&av.full_id);
        Some(compare(appearance, self.listed_attachments(&av.full_id), &worn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn everything_listed_came() {
        let r = compare(true, &[(id(1), 2), (id(2), 5)], &[(id(1), None), (id(2), Some(id(20)))]);
        assert_eq!((r.listed, r.arrived, r.arrived_listed), (2, 2, 2));
        assert!(r.missing.is_empty());
        assert_eq!(
            r.line(),
            "worn objects: 2 listed by the simulator, 2 arrived (2 of the listed, 0 not listed), none missing"
        );
    }

    #[test]
    fn listed_objects_that_never_came_are_named() {
        let r = compare(true, &[(id(1), 2), (id(2), 5), (id(3), 39)], &[(id(1), None), (id(9), None)]);
        assert_eq!((r.listed, r.arrived, r.arrived_listed), (3, 2, 1));
        assert_eq!(r.missing, vec![(id(2), 5), (id(3), 39)]);
        assert!(
            r.line()
                .ends_with(&format!("2 missing: {} on point 5, {} on point 39", id(2), id(3)))
        );
    }

    #[test]
    fn a_list_of_items_still_matches_by_attach_item() {
        let r = compare(true, &[(id(20), 2)], &[(id(1), Some(id(20)))]);
        assert_eq!((r.arrived_listed, r.missing.len()), (1, 0));
    }

    #[test]
    fn without_an_appearance_or_a_list() {
        assert_eq!(
            compare(false, &[], &[(id(1), None)]).line(),
            "worn objects: no appearance received yet, 1 arrived"
        );
        let r = compare(true, &[], &[]);
        assert_eq!(
            r.line(),
            "worn objects: 0 listed by the simulator, 0 arrived (0 of the listed, 0 not listed), none missing"
        );
    }
}
