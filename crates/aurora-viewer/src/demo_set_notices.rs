//! AURORA_DEMO_SET_NOTICES=config|config-responses|config-bottom|chat: a contact set's own online notices
//! and automatic responses (world/contact_sets/notices.rs). At frame 60 the
//! set Family (the demo friend in it, kept in memory) gets "Show
//! notifications" and its own do not disturb response, and the friends'
//! global notices are off for the run. `config` opens its settings window
//! (`config-responses` scrolled down to the set's responses,
//! `config-bottom` to the choices of all the sets);
//! `chat` shows nearby chat, where the demo friend going offline and online
//! (frames 300 and 360) gets a line and another friend none; at 420, in do
//! not disturb, the demo friend's message gets the set's response (logged).

use super::{DEMO_FRIEND, u};
use crate::world::World;
use crate::world::contact_sets::{ResponseMode, SetResponse};
use aurora_net::{InstantMessage, NetEvent};
use uuid::Uuid;

pub const SETUP: u64 = 60;
const OFFLINE: u64 = 300;
const ONLINE: u64 = 360;
pub const DND: u64 = 400;
pub const MESSAGE: u64 = 420;

pub fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_SET_NOTICES").ok()
}

pub fn active() -> bool {
    mode().is_some()
}

/// The set, its notices and its response.
pub fn setup(world: &mut World) {
    let sets = &mut world.contact_sets;
    sets.keep_in_memory();
    let family = aurora_i18n::tr!("demo-family");
    sets.add_set(family);
    sets.set_color(family, [0.957, 0.447, 0.714, 1.0]);
    sets.add_to_set(&[DEMO_FRIEND], family, |_| true);
    sets.set_notify(family, true);
    let response = SetResponse {
        enabled: true,
        text: aurora_i18n::tr!("demo-set-notices-busy").into(),
    };
    sets.set_response(family, ResponseMode::Busy, response);
}

/// The events of `frame`.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    match frame {
        OFFLINE | ONLINE => vec![NetEvent::FriendsOnline {
            // u(101): a demo friend in no set
            ids: vec![DEMO_FRIEND, u(101)],
            online: frame == ONLINE,
        }],
        MESSAGE => vec![NetEvent::InstantMessage(InstantMessage {
            from_agent_id: DEMO_FRIEND,
            from_name: "Nova Example".into(),
            to_agent_id: agent,
            dialog: 0,
            session_id: Uuid::from_u128(agent.as_u128() ^ DEMO_FRIEND.as_u128()),
            message: aurora_i18n::tr!("demo-im-announce-message").into(),
            offline: false,
            from_group: false,
            binary_bucket: vec![0],
            timestamp: 0,
        })],
        _ => Vec::new(),
    }
}
