//! Map and tag privacy parity with the Legacy Viewer (names on tags,
//! hover texts, map and nearby lists).

use super::*;

const A: Uuid = Uuid::from_u128(1);
const B: Uuid = Uuid::from_u128(2);
const TARGET: Uuid = Uuid::from_u128(3);
const OWN: Uuid = Uuid::from_u128(4);

fn command(r: &mut Rlva, source: Uuid, text: &str) {
    let c = ChatMessage {
        from_name: "RLVa test object".to_owned(),
        source_id: source,
        owner_id: OWN,
        source_type: ChatSourceType::Object,
        chat_type: ChatType::OwnerSay,
        region: None,
        position: glam::Vec3::ZERO,
        message: text.to_owned(),
    };
    assert!(r.receive(&c));
}

fn enabled() -> Rlva {
    let mut r = Rlva::default();
    r.configure(true);
    r
}

#[test]
fn rlva_plain_shownametags_cancels_every_distance_like_the_legacy_minimum_modifier() {
    let mut r = enabled();
    command(&mut r, A, "@shownametags:5=n");
    assert!(r.nametag_visible(TARGET, OWN, 3.0), "inside the distance the tag shows");
    assert!(!r.nametag_visible(TARGET, OWN, 6.0));
    command(&mut r, B, "@shownametags=n");
    assert!(
        !r.nametag_visible(TARGET, OWN, 3.0),
        "a plain rule adds the modifier default 0, and the smallest value wins"
    );
    assert!(r.nametag_visible(OWN, OWN, 0.0), "the own tag always shows");
    command(&mut r, B, &format!("@shownametags:{TARGET}=n"));
    assert!(r.nametag_visible(TARGET, OWN, 30.0), "an exception still names the avatar");
    command(&mut r, B, "@clear");
    assert!(
        r.nametag_visible(TARGET, OWN, 3.0),
        "releasing the plain rule restores the distance"
    );
}
