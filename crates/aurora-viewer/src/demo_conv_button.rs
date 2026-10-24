//! AURORA_DEMO_CONV_BUTTON: the Conversations button's flash, the
//! Conversations window closed. Tess says something in nearby chat at frame
//! 700, Nova writes an IM at 900. `1`: both flash the button; `mute`: nearby
//! chat left out for the run (never saved), only the IM flashes; `still`:
//! Reduce motion for the run, the button lights without its rises; `menu`:
//! the button's right-click menu opens at frame 700.

use aurora_net::NetEvent;
use glam::Vec3;
use uuid::Uuid;

/// The scenario, if asked: `1`, `mute`, `still` or `menu`.
pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_CONV_BUTTON").ok()
}

/// Nearby chat left out of the flash for the run (`mute`).
pub fn nearby_off() -> bool {
    mode().as_deref() == Some("mute")
}

/// Reduce motion for the run (`still`).
pub fn still() -> bool {
    mode().as_deref() == Some("still")
}

/// Frame at which the right-click menu opens (`menu`).
pub const MENU_AT: u64 = 700;

/// `menu`: a right click on the chat island's Conversations button.
pub fn input_events(ctx: &egui::Context, frame: u64) -> Vec<egui::Event> {
    if mode().as_deref() != Some("menu") || !(MENU_AT..=MENU_AT + 1).contains(&frame) {
        return Vec::new();
    }
    let Some(button) = ctx.read_response(egui::Id::unique("chat_island_open")) else {
        return Vec::new();
    };
    let pos = button.rect.center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: Default::default(),
    };
    if frame == MENU_AT {
        vec![egui::Event::PointerMoved(pos), press(true)]
    } else {
        vec![press(false)]
    }
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_NOVA, DEMO_TESS};
    match frame {
        700 => vec![NetEvent::Chat(aurora_net::ChatMessage {
            from_name: "Tess Touch".into(),
            source_id: DEMO_TESS,
            owner_id: Uuid::nil(),
            source_type: aurora_net::ChatSourceType::Agent,
            chat_type: aurora_net::ChatType::Normal,
            position: Vec3::ZERO,
            message: aurora_i18n::tr!("demo-toast-hello").into(),
        })],
        900 => vec![NetEvent::InstantMessage(aurora_net::InstantMessage {
            from_agent_id: DEMO_NOVA,
            from_name: "Nova Exemple".into(),
            to_agent_id: agent,
            dialog: 0,
            session_id: Uuid::nil(),
            message: aurora_i18n::tr!("demo-toast-are-you-there").into(),
            offline: false,
            from_group: false,
            binary_bucket: Vec::new(),
            timestamp: 0,
        })],
        _ => Vec::new(),
    }
}
