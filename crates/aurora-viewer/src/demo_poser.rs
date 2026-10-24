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
