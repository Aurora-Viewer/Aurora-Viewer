//! Demo simulator for the small simulator features of #660, without a grid:
//!
//! - `AURORA_DEMO_FREEZE=<seconds>` (`1`: 30 s): at frame 660 Nova freezes
//!   the avatar (ViewerFrozenMessage and its AvatarFrozenDuration alert),
//!   and releases it after that many seconds at 60 fps.
//! - `AURORA_DEMO_SIM_CONSOLE=1`: the region gives the console
//!   (SimConsoleAsync) and answers `help`, `stats` and the rest; `open`
//!   also opens it at frame 600 with `help` and `stats` typed.
//! - `AURORA_DEMO_VOICE_MODERATION=1`: the avatar manages the estate, whose
//!   voice channel the parcel uses, so the voice row of a resident's menu
//!   offers the moderator's entries; the demo applies them. `self`: a
//!   moderator mutes us at frame 700 and lets us speak again at 1300.

use aurora_net::NetEvent;
use aurora_net::freeze::Alert;
use aurora_net::voice_moderation::Target;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

/// The frame the demo freezes the avatar.
const FREEZE_FRAME: u64 = 660;

/// AURORA_DEMO_FREEZE: how long the freeze lasts, in seconds.
fn freeze_seconds() -> Option<i32> {
    static SECONDS: OnceLock<Option<i32>> = OnceLock::new();
    *SECONDS.get_or_init(|| match std::env::var("AURORA_DEMO_FREEZE").ok()?.trim() {
        "" | "0" => None,
        "1" => Some(30),
        n => n.parse().ok().filter(|s: &i32| *s > 0),
    })
}

/// The freeze and its release, at their frames.
pub fn freeze_events(frame: u64) -> Vec<NetEvent> {
    let Some(seconds) = freeze_seconds() else {
        return Vec::new();
    };
    let by = "Nova Demo".to_owned();
    if frame == FREEZE_FRAME {
        vec![
            NetEvent::Frozen(true),
            NetEvent::FreezeAlert(Alert::Frozen {
                by,
                seconds: Some(seconds),
            }),
        ]
    } else if frame == FREEZE_FRAME + seconds as u64 * 60 {
        vec![NetEvent::Frozen(false), NetEvent::FreezeAlert(Alert::Unfrozen { by })]
    } else {
        Vec::new()
    }
}

fn console() -> bool {
    std::env::var("AURORA_DEMO_SIM_CONSOLE").is_ok_and(|v| matches!(v.trim(), "1" | "open"))
}

/// AURORA_DEMO_SIM_CONSOLE=open: the console opened at frame 600, `help`
/// and `stats` typed.
pub fn console_open() -> bool {
    std::env::var("AURORA_DEMO_SIM_CONSOLE").is_ok_and(|v| v.trim() == "open")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Moderation {
    /// We moderate the others.
    Moderator,
    /// A moderator mutes us.
    MutedByModerator,
}

fn moderation() -> Option<Moderation> {
    match std::env::var("AURORA_DEMO_VOICE_MODERATION").ok()?.trim() {
        "1" => Some(Moderation::Moderator),
        "self" => Some(Moderation::MutedByModerator),
        _ => None,
    }
}

/// The avatar manages the estate (the moderator of the estate's voice).
pub fn estate_manager() -> bool {
    moderation() == Some(Moderation::Moderator)
}

/// The parcel uses the estate's voice channel (USE_ESTATE_VOICE_CHAN).
pub fn estate_voice() -> bool {
    estate_manager()
}

/// The capabilities the scenarios add to the demo region.
pub fn caps() -> Vec<(String, String)> {
    let mut caps = Vec::new();
    if console() {
        caps.push(("SimConsoleAsync".to_owned(), "demo://console".to_owned()));
    }
    if moderation().is_some() {
        caps.push(("SpatialVoiceModerationRequest".to_owned(), "demo://voice-moderation".to_owned()));
    }
    caps
}

/// The console's output for a command line (SimConsoleResponse).
pub fn console_reply(command: &str) -> NetEvent {
    let text = match command.trim() {
        "help" => "Commands:\n  help   this list\n  stats  simulator statistics\n  time   the simulator's clock".to_owned(),
        "stats" => "Agents: 3\nPrimitives: 2,140 / 15,000\nScript time: 1.82 ms\nFrame time: 22.20 ms".to_owned(),
        "time" => "2026-10-21 12:00:00 SLT".to_owned(),
        other => format!("Unknown command: {other}"),
    };
    NetEvent::SimConsoleReply(Ok(text))
}

/// Who the demo's moderator muted, by resident.
fn muted() -> &'static Mutex<HashMap<Uuid, bool>> {
    static MUTED: OnceLock<Mutex<HashMap<Uuid, bool>>> = OnceLock::new();
    MUTED.get_or_init(Default::default)
}

/// A moderation request: applied at once, as the simulator would.
pub fn moderate(target: Target, mute: bool, peers: &[Uuid]) {
    let mut m = muted().lock().unwrap_or_else(|e| e.into_inner());
    match target {
        Target::Resident(id) => {
            m.insert(id, mute);
        }
        Target::Everyone => {
            for id in peers {
                m.insert(*id, mute);
            }
        }
    }
}

/// The moderator mute of the nearby channel's participants (us included),
/// as the voice server's data channel would give it; None without the
/// scenario.
pub fn nearby_moderated(frame: u64, me: Uuid, peers: impl Iterator<Item = Uuid>) -> Option<HashMap<Uuid, bool>> {
    let mode = moderation()?;
    let m = muted().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: HashMap<Uuid, bool> = peers.map(|id| (id, m.get(&id).copied().unwrap_or(false))).collect();
    let self_muted = mode == Moderation::MutedByModerator && (700..1300).contains(&frame);
    out.insert(me, self_muted);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_console_answers_every_command() {
        for (command, expected) in [
            ("help", "Commands:"),
            (" stats ", "Agents: 3"),
            ("reboot", "Unknown command: reboot"),
        ] {
            match console_reply(command) {
                NetEvent::SimConsoleReply(Ok(text)) => assert!(text.starts_with(expected), "{command}: {text}"),
                other => panic!("{other:?}"),
            }
        }
    }
}
