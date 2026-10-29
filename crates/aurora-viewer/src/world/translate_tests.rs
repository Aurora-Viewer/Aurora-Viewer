//! The World's side of the IM translation: incoming messages handed to the
//! app, our lines found again by their time, translated lines replaced.

use super::*;
use crate::translate::LineKey;
use aurora_net::InstantMessage;

const ME: Uuid = Uuid::from_u128(1);
const BOB: Uuid = Uuid::from_u128(0xB0B);

fn world() -> World {
    let mut w = World::new(Arc::new(AvatarLibrary::load()));
    w.agent_id = ME;
    w.social.names.insert(BOB, "Bob Ross".into());
    w
}

fn im(text: &str, offline: bool) -> InstantMessage {
    InstantMessage {
        from_agent_id: BOB,
        from_name: "Bob Ross".into(),
        to_agent_id: ME,
        dialog: 0,
        session_id: BOB,
        message: text.into(),
        offline,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: if offline { 1_700_000_000 } else { 0 },
    }
}

fn lines(w: &mut World) -> Vec<String> {
    w.session_lines(BOB).lines.iter().map(|l| l.text.clone()).collect()
}

#[test]
fn incoming_private_ims_are_handed_to_the_app() {
    let mut w = world();
    w.on_im(im("Hola amigo", false));
    let got = std::mem::take(&mut w.incoming_ims);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].0, got[0].2.as_str()), (BOB, "Hola amigo"));
}

#[test]
fn saved_prefix_is_kept_out_of_the_job() {
    let mut w = world();
    w.on_im(im("Hola amigo", true));
    let got = std::mem::take(&mut w.incoming_ims);
    assert_eq!(got[0].2, "Hola amigo");
    assert!(lines(&mut w)[0].ends_with("Hola amigo") && lines(&mut w)[0] != "Hola amigo");
}

#[test]
fn own_im_returns_the_line_time() {
    let mut w = world();
    let time = w.own_im(BOB, "hello");
    assert!(w.session_lines(BOB).lines.iter().any(|l| l.time == time && l.text == "hello"));
}

#[test]
fn replace_line_changes_the_text_and_keeps_the_prefix() {
    let mut w = world();
    w.on_im(im("Hola amigo", true));
    let (session, time, _) = w.incoming_ims.remove(0);
    w.replace_line(&LineKey { session, time }, "Hola amigo", "Hello friend");
    let l = lines(&mut w);
    assert!(l[0].ends_with("Hello friend") && l[0] != "Hello friend", "{l:?}");
    let time = w.own_im(BOB, "hello");
    w.replace_line(&LineKey { session: BOB, time }, "hello", "hola");
    assert_eq!(lines(&mut w)[1], "hola");
}

#[test]
fn recent_incoming_lists_their_last_messages_oldest_first() {
    let mut w = world();
    for i in 0..7 {
        w.on_im(im(&format!("mensaje {i}"), false));
    }
    w.own_im(BOB, "hello");
    w.incoming_ims.clear();
    let got: Vec<String> = w.recent_incoming(BOB, 5).into_iter().map(|(_, t)| t).collect();
    assert_eq!(got, ["mensaje 2", "mensaje 3", "mensaje 4", "mensaje 5", "mensaje 6"]);
}
