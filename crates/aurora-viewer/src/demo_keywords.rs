//! AURORA_DEMO_KEYWORDS: keyword alerts on for the run (never saved), with
//! two keywords, their colour and their sound; then a line of nearby chat,
//! a private message, a group chat line and an object's message holding a
//! keyword arrive one second apart, and a line without one. Each keyword
//! line is coloured, plays the sound and, with the system notifications on,
//! is told as a "Keyword alert". `prefs` keeps the settings of the
//! preferences (to try them by hand) and sends the same lines again every
//! `PERIOD` frames.

use crate::keywords::KeywordSettings;
use aurora_net::{ChatMessage, ChatSourceType, ChatType, InstantMessage, NetEvent};
use uuid::Uuid;

/// Frames between two rounds of lines in `prefs`.
const PERIOD: u64 = 1200;

fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_KEYWORDS").ok()
}

/// The scenario is asked.
pub fn active() -> bool {
    mode().is_some()
}

/// The settings of the run, unless `prefs` keeps the preferences'.
pub fn settings() -> Option<KeywordSettings> {
    mode().filter(|m| m != "prefs").map(|_| KeywordSettings {
        on: true,
        in_chat: true,
        in_im: true,
        words: aurora_i18n::tr!("demo-keywords-words").to_owned(),
        change_color: true,
        play_sound: true,
        ..Default::default()
    })
}

const OBJECT: Uuid = Uuid::from_u128(0x0E1C_0000_0000_0000_0000_0000_0000_0001);

fn im(agent: Uuid, from: Uuid, name: &str, dialog: u8, session: Uuid, text: &str, bucket: &[u8]) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: from,
        from_name: name.into(),
        to_agent_id: agent,
        dialog,
        session_id: session,
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: bucket.to_vec(),
        timestamp: 0,
    })
}

fn said(text: &str) -> NetEvent {
    NetEvent::Chat(ChatMessage {
        from_name: "Tess Touch".into(),
        source_id: super::DEMO_TESS,
        owner_id: super::DEMO_TESS,
        source_type: ChatSourceType::Agent,
        chat_type: ChatType::Normal,
        position: glam::Vec3::new(130.0, 130.0, 22.0),
        message: text.into(),
    })
}

/// The events of `frame` (after the loading fade).
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    use super::{DEMO_GROUP1, DEMO_NOVA, DEMO_TESS};
    let frame = if frame > 700 && mode().as_deref() == Some("prefs") {
        700 + (frame - 700) % PERIOD
    } else {
        frame
    };
    let event = match frame {
        700 => said(aurora_i18n::tr!("demo-keywords-nearby")),
        760 => im(
            agent,
            DEMO_NOVA,
            "Nova Exemple",
            0,
            Uuid::nil(),
            aurora_i18n::tr!("demo-keywords-im"),
            b"",
        ),
        820 => im(
            agent,
            DEMO_TESS,
            "Tess Touch",
            17,
            DEMO_GROUP1,
            aurora_i18n::tr!("demo-keywords-group"),
            b"Aurora Builders\0",
        ),
        880 => {
            let location = format!("{}/128/128/25\0", aurora_i18n::tr!("common-aurora-demo"));
            im(
                agent,
                agent,
                aurora_i18n::tr!("demo-keywords-object-name"),
                19,
                OBJECT,
                aurora_i18n::tr!("demo-keywords-object"),
                location.as_bytes(),
            )
        }
        940 => said(aurora_i18n::tr!("demo-keywords-plain")),
        _ => return Vec::new(),
    };
    vec![event]
}
