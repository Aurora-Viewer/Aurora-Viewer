//! The staff Aurora marks: the Aurora team, with an aurora on their name
//! plate (`ui/nameplate/aurora.rs`), and Linden Lab's employees, with
//! twinkling stars and a running shine (`ui/nameplate/linden.rs`); each
//! with its tag next to their name in the lists, the chat and their
//! profile (`ui/staff.rs`). Only Aurora draws them: the team's list ships
//! with the viewer and nothing is sent to the grid. Aurora's own; the
//! Legacy Viewer only colours Linden names (`ui::colors::is_linden`).

use super::World;
use uuid::Uuid;

/// The team's avatar keys.
const TEAM: &[Uuid] = &[
    uuid::uuid!("348d7782-8438-4480-b482-608ca731b3a8"),
    uuid::uuid!("eb902302-0a1a-4f51-9ca6-7148822d9e95"),
    uuid::uuid!("322be05d-8851-4eaf-b4f2-a3347ebde023"),
    uuid::uuid!("2401c1a6-0dde-4daf-b60f-420473687744"),
    uuid::uuid!("d94d9882-b0bb-400b-9299-e51e021e921e"),
];

/// Whose staff a resident is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// The Aurora team.
    Aurora,
    /// A Linden Lab employee: the last name "Linden" (LLMuteList::isLinden).
    Linden,
}

/// Whether `id` is a member of the Aurora team.
pub fn is_staff(id: &Uuid) -> bool {
    !id.is_nil() && (TEAM.contains(id) || crate::demo::staff::is_staff(id))
}

/// The mark of `id`, if any: the Aurora team by key; Linden Lab by the
/// legacy name of the cached avatar name (a display name can say anything).
pub fn mark(world: &World, id: &Uuid) -> Option<Mark> {
    if is_staff(id) {
        return Some(Mark::Aurora);
    }
    let names = &world.social.avatar_names;
    let linden = names
        .get(id)
        .is_some_and(|n| super::mutes::is_linden(&n.user_name_for_display(&names.options)));
    (linden || crate::demo::staff::is_linden(id)).then_some(Mark::Linden)
}

/// The mark of a resident whose legacy name is `legacy` (the name plates,
/// which already have it).
pub fn mark_named(id: &Uuid, legacy: &str) -> Option<Mark> {
    if is_staff(id) {
        Some(Mark::Aurora)
    } else if super::mutes::is_linden(legacy) || crate::demo::staff::is_linden(id) {
        Some(Mark::Linden)
    } else {
        None
    }
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

    #[test]
    fn the_team_is_marked_by_key() {
        assert!(TEAM.iter().all(is_staff));
        for (i, id) in TEAM.iter().enumerate() {
            assert!(!TEAM[i + 1..].contains(id), "{id} twice");
            // whatever their name says
            assert_eq!(mark_named(id, "Someone Linden"), Some(Mark::Aurora));
        }
    }

    #[test]
    fn lindens_are_marked_by_their_last_name() {
        let id = Uuid::from_u128(7);
        assert_eq!(mark_named(&id, "Philip Linden"), Some(Mark::Linden));
        assert_eq!(mark_named(&id, "philip.linden"), Some(Mark::Linden));
        assert_eq!(mark_named(&id, "Lindenberg Resident"), None);
        assert_eq!(mark_named(&id, "Nova Example"), None);
    }
}
