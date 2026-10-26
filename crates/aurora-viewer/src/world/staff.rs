//! The Aurora team: the avatars Aurora marks as its staff, with an aurora
//! on their name plate (`ui/nameplate/aurora.rs`) and the "Aurora Staff"
//! tag next to their name in the lists, the chat and their profile
//! (`ui/staff.rs`). Only Aurora draws it: the list ships with the viewer
//! and nothing is sent to the grid. Aurora's own; the Legacy Viewer marks
//! Linden Lab staff only (by their last name, `ui::colors::is_linden`).

use uuid::Uuid;

/// The team's avatar keys.
const TEAM: &[Uuid] = &[];

/// Whether `id` is a member of the Aurora team.
pub fn is_staff(id: &Uuid) -> bool {
    !id.is_nil() && (TEAM.contains(id) || crate::demo::staff::is_staff(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_is_staff_by_default() {
        assert!(!is_staff(&Uuid::nil()));
        assert!(!is_staff(&Uuid::from_u128(1)));
        assert!(TEAM.iter().all(|id| !id.is_nil()));
    }
}
