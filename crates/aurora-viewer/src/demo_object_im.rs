//! AURORA_DEMO_OBJECT_IM=off|region|parcel|strict: objects' messages from
//! near and far (`world/object_im_filter.rs`), with the filter set for this
//! run only: `off`, on for the region, on for the parcel, or on for the
//! parcel with friends' objects blocked too. Seven messages reach nearby
//! chat from frame 700, each saying when it should be hidden; the log has
//! one "dropped" line for each hidden one.

use crate::world::object_im_filter::{Scope, Settings};
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent};
use glam::Vec3;
use uuid::Uuid;

/// The filter of this run, if the scenario is on.
pub fn settings() -> Option<Settings> {
    let mode = std::env::var("AURORA_DEMO_OBJECT_IM").ok()?;
    let on = |scope, allow_friends| Settings {
        on: true,
        scope,
        allow_friends,
    };
    Some(match mode.trim() {
        "off" => Settings::default(),
        "parcel" => on(Scope::Parcel, true),
        "strict" => on(Scope::Parcel, false),
        _ => on(Scope::Region, true),
    })
}

/// An owner who is neither us nor a friend.
const STRANGER: Uuid = Uuid::from_u128(0x0B1E_C700_0000_0000_0000_0000_0000_0001);

fn object(n: u128) -> Uuid {
    Uuid::from_u128(0x0B1E_C700_0000_0000_0000_0000_0000_0100 | n)
}

fn name(n: u128) -> String {
    aurora_i18n::tr!("demo-object-im-name", n = n.to_string()).to_string()
}

/// llInstantMessage from `owner`'s object at `location` ("Region/x/y/z").
fn im(n: u128, owner: Uuid, agent: Uuid, location: &str, text: &str) -> NetEvent {
    let mut bucket = location.as_bytes().to_vec();
    bucket.push(0);
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: owner,
        from_name: name(n),
        to_agent_id: agent,
        dialog: 19,
        session_id: object(n),
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket,
        timestamp: 0,
    })
}

/// The events of `frame`. The demo parcel is the square 64..192 m.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    let here = aurora_i18n::tr!("common-aurora-demo");
    let ev = match frame {
        700 => im(
            1,
            STRANGER,
            agent,
            "Far Away/128/128/25",
            aurora_i18n::tr!("demo-object-im-other-region"),
        ),
        730 => im(2, agent, agent, "Far Away/30/40/25", aurora_i18n::tr!("demo-object-im-own")),
        760 => im(
            3,
            super::DEMO_FRIEND,
            agent,
            "Far Away/60/90/25",
            aurora_i18n::tr!("demo-object-im-friend"),
        ),
        790 => im(
            4,
            STRANGER,
            agent,
            &format!("{here}/20/20/21"),
            aurora_i18n::tr!("demo-object-im-other-parcel"),
        ),
        820 => im(
            5,
            STRANGER,
            agent,
            &format!("{here}/128/128/21"),
            aurora_i18n::tr!("demo-object-im-this-parcel"),
        ),
        // llRegionSayTo from outside the parcel
        850 => NetEvent::Chat(ChatMessage {
            from_name: name(6),
            source_id: object(6),
            owner_id: STRANGER,
            source_type: ChatSourceType::Object,
            chat_type: ChatType::Other(9),
            position: Vec3::new(230.0, 30.0, 21.0),
            message: aurora_i18n::tr!("demo-object-im-say-to").into(),
        }),
        880 => im(7, STRANGER, agent, "", aurora_i18n::tr!("demo-object-im-no-location")),
        _ => return Vec::new(),
    };
    vec![ev]
}
