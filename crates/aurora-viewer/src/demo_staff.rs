//! AURORA_DEMO_STAFF=1: a few demo residents count as members of the Aurora
//! team or as Linden Lab employees (`world::staff`), to see their marks
//! without the real avatars.
//!
//! The Aurora team: ourselves, the first friend of the buddy list (the
//! profile and chat scenarios), the resident who speaks in nearby chat in
//! the bubbles scenario and the chat console (`DEMO_NOVA`), and with
//! AURORA_DEMO_TAGS Orion (full plate, speaking), Sol (compact pill) and
//! Vega (far pill). Linden Lab: the wolf (`DEMO_LOUP`: nearby chat, his
//! profile), Lune of People › Near you, and with AURORA_DEMO_TAGS Lune
//! (full plate, typing) and Iris (compact pill).

use uuid::Uuid;

pub fn active() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("AURORA_DEMO").is_some() && std::env::var("AURORA_DEMO_STAFF").is_ok_and(|v| !matches!(v.trim(), "" | "0"))
    })
}

/// Whether `id` is one of the scenario's Aurora team members.
pub fn is_staff(id: &Uuid) -> bool {
    active() && team().contains(id)
}

/// Whether `id` is one of the scenario's Linden Lab employees.
pub fn is_linden(id: &Uuid) -> bool {
    active() && lindens().contains(id)
}

fn tags(n: u128) -> Uuid {
    Uuid::from_u128(0xA0E0_7A65_0000_0000_0000_0000_0000_0000 | n)
}

fn team() -> [Uuid; 6] {
    [
        super::DEMO_AGENT,
        super::DEMO_FRIEND,
        super::DEMO_NOVA,
        // Orion of the name plates scenario, a friend of the buddy list
        Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | 102),
        tags(6),
        tags(8),
    ]
}

fn lindens() -> [Uuid; 4] {
    [
        super::DEMO_LOUP,
        // Lune of People › Near you (demo_people.rs)
        Uuid::from_u128(0xA0E0_9E09_0000_0000_0000_0000_0000_0001),
        tags(4),
        tags(7),
    ]
}
