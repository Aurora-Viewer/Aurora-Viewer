//! AURORA_DEMO_TRANSLATE: the translation of private conversations
//! (`crate::translate`) without a grid nor a model. A resident writes two
//! Spanish messages in a private conversation: with `1` the preferences
//! decide (translation off by default: nothing happens; on: the suggestion
//! to translate it shows); with `on` translation is on and the conversation
//! translated from the start, the resident writes again and we answer in
//! English. The model is a fake one that
//! answers "[<language>] <text>" after 0.4 s, unless AURORA_TRANSLATE_REAL is
//! set: then the real model (downloaded first if needed).

use aurora_net::{InstantMessage, NetEvent};
use uuid::Uuid;

fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_TRANSLATE").ok()
}

pub fn active() -> bool {
    mode().is_some()
}

/// The conversation starts translated to Spanish.
pub fn starts_on() -> bool {
    mode().as_deref() == Some("on")
}

/// The real model instead of the fake one.
pub fn real() -> bool {
    std::env::var_os("AURORA_TRANSLATE_REAL").is_some()
}

/// The fake model of the demo.
pub fn backend() -> Box<dyn aurora_translate::Backend> {
    Box::new(aurora_translate::FakeBackend::slow(std::time::Duration::from_millis(400)))
}

/// The conversation of the scenario.
pub const WITH: Uuid = super::DEMO_NOVA;

/// Our answer, sent at this frame with `on`.
pub const ANSWER_FRAME: u64 = 1000;
pub const ANSWER: &str = "Sounds great, see you at the beach in five minutes!";

fn im(agent: Uuid, text: &str) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: WITH,
        from_name: "Nova Exemple".into(),
        to_agent_id: agent,
        dialog: 0,
        session_id: Uuid::nil(),
        message: text.into(),
        offline: false,
        from_group: false,
        binary_bucket: Vec::new(),
        timestamp: 0,
    })
}

/// The events of `frame` (after the loading fade).
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    let text = match frame {
        700 => "¡Hola! ¿Cómo estás hoy? Te estaba buscando en la plaza.",
        760 => "¿Quieres ir a la playa conmigo esta tarde?",
        900 if starts_on() => "Perfecto, te espero cerca del muelle.",
        _ => return Vec::new(),
    };
    vec![im(agent, text)]
}
