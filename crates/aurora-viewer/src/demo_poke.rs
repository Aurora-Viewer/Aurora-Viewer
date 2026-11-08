//! AURORA_DEMO_POKE=<n>: from frame 700, Nova pokes us `n` times (3 when
//! the value is not a number), 120 frames apart: her poke card comes out,
//! jiggles at each poke and counts them. A poke we send to a demo resident
//! (a friend's row, a conversation's head, a profile, Poke back) is
//! answered by a poke of theirs `ANSWER_AFTER` later, so that a poke war
//! can be played offline (`world/pokes.rs`).

use aurora_net::{InstantMessage, NetCommand, NetEvent};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The first poke, and the frames between two.
const START: u64 = 700;
const EVERY: u64 = 120;
/// A demo resident pokes back this long after our poke.
const ANSWER_AFTER: Duration = Duration::from_millis(1500);

/// The pokes back to come: who, and when.
static ANSWERS: Mutex<Vec<(Uuid, Instant)>> = Mutex::new(Vec::new());

fn mode() -> Option<String> {
    std::env::var("AURORA_DEMO_POKE").ok()
}

/// The scenario runs.
pub fn active() -> bool {
    mode().is_some()
}

/// How many times Nova pokes.
fn count() -> u64 {
    mode().and_then(|m| m.trim().parse().ok()).unwrap_or(3)
}

fn name_of(id: Uuid) -> &'static str {
    use super::{DEMO_NOVA, DEMO_TESS};
    match id {
        DEMO_NOVA => "Nova Exemple",
        DEMO_TESS => "Tess Touch",
        _ => "Demo Resident",
    }
}

fn poke(from: Uuid, agent: Uuid) -> NetEvent {
    NetEvent::InstantMessage(InstantMessage {
        from_agent_id: from,
        from_name: name_of(from).into(),
        to_agent_id: agent,
        dialog: 0,
        session_id: Uuid::from_u128(agent.as_u128() ^ from.as_u128()),
        message: crate::world::pokes::POKE.to_owned(),
        offline: false,
        from_group: false,
        binary_bucket: vec![0],
        timestamp: 0,
    })
}

/// Our poke to a demo resident: their poke back comes `ANSWER_AFTER` later.
pub fn sent(cmd: &NetCommand) {
    if !active() {
        return;
    }
    if let NetCommand::SendIm { to, message } = cmd
        && crate::world::pokes::is_poke(message)
        && let Ok(mut answers) = ANSWERS.lock()
    {
        answers.push((*to, Instant::now() + ANSWER_AFTER));
    }
}

/// Nova's pokes at their frames.
fn scheduled(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    match frame.checked_sub(START) {
        Some(since) if since.is_multiple_of(EVERY) && since / EVERY < count() => vec![poke(super::DEMO_NOVA, agent)],
        _ => Vec::new(),
    }
}

/// The events of `frame`: Nova's pokes, and the pokes back now due.
pub fn events(frame: u64, agent: Uuid) -> Vec<NetEvent> {
    let mut out = scheduled(frame, agent);
    let now = Instant::now();
    if let Ok(mut answers) = ANSWERS.lock() {
        answers.retain(|(from, at)| {
            let due = *at <= now;
            if due {
                out.push(poke(*from, agent));
            }
            !due
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pokers(frame: u64) -> Vec<Uuid> {
        scheduled(frame, Uuid::from_u128(1))
            .into_iter()
            .filter_map(|e| match e {
                NetEvent::InstantMessage(im) if crate::world::pokes::is_poke(&im.message) => Some(im.from_agent_id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nova_pokes_three_times_by_default() {
        use crate::demo::DEMO_NOVA;
        assert!(pokers(START - 1).is_empty());
        assert_eq!(pokers(START), vec![DEMO_NOVA]);
        assert!(pokers(START + 1).is_empty());
        assert_eq!(pokers(START + EVERY), vec![DEMO_NOVA]);
        assert_eq!(pokers(START + 2 * EVERY), vec![DEMO_NOVA]);
        assert!(pokers(START + 3 * EVERY).is_empty(), "three pokes only");
    }
}
