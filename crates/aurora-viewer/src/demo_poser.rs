//! A deterministic nearby resident for the offline consent scenarios and tests.
use super::*;
pub const PEER: Uuid = Uuid::from_u128(0x727_aaaa);
pub fn peer() -> ObjectUpdate {
    let mut object = action_avatar(false);
    object.full_id = PEER;
    object.local_id = 9727;
    object.position += Vec3::new(1.5, 0.5, 0.0);
    object.name_values = "FirstName STRING RW SV Nova\nLastName STRING RW SV Aurora".into();
    object
}

/// AURORA_DEMO_POSER=compact-crowd: five more residents around you, each
/// with a display name and an account (short to long), who all accept.
pub const CROWD: [(&str, &str, &str); 5] = [
    ("Lumi", "lumi.starling", "Starling"),
    ("Kai Moonshadow", "kai.moonshadow", "Moonshadow"),
    ("Seraphina Delacroix-Vale", "seraphinadelacroix.vale", "Vale"),
    ("Bo", "bo.resident", "Resident"),
    ("Tomasz Wierzbicki", "tomaszwierzbicki", "Resident"),
];
pub fn crowd_id(i: usize) -> Uuid {
    Uuid::from_u128(0x727_bbb0 + i as u128)
}
pub fn crowd() -> Vec<ObjectUpdate> {
    (0..CROWD.len())
        .map(|i| {
            let mut object = action_avatar(false);
            object.full_id = crowd_id(i);
            object.local_id = 9740 + i as u32;
            // an arc in front of you, 2 to 7 m away
            let angle = -0.9 + 0.45 * i as f32;
            let distance = 2.0 + 1.2 * i as f32;
            object.position += Vec3::new(angle.cos() * distance, angle.sin() * distance, 0.0);
            let (display, _, last) = CROWD[i];
            let first = display.split(' ').next().unwrap_or(display);
            object.name_values = format!("FirstName STRING RW SV {first}\nLastName STRING RW SV {last}");
            object
        })
        .collect()
}
