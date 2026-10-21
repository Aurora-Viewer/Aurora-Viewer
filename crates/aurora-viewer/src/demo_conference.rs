//! AURORA_DEMO_CONFERENCE[=<seconds>]: a conference invitation every
//! `<seconds>` (20 by default), from a stranger then from a friend in turn,
//! to try Preferences › Chat › Ignore conferences and Online status › Ad hoc
//! conferences while the viewer runs: an ignored invitation never opens a
//! conversation, its report names the inviter in nearby chat.

use super::*;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// First session of the scenario; each invitation takes the next one.
const SESSION: u128 = 0xC0F0_0000_0000_0000_0000_0000_0000_0000;

fn every() -> Option<f32> {
    static EVERY: OnceLock<Option<f32>> = OnceLock::new();
    *EVERY.get_or_init(|| {
        let v = std::env::var("AURORA_DEMO_CONFERENCE").ok()?;
        // "1" turns it on at the default period
        Some(v.parse::<f32>().ok().filter(|s| *s > 1.0).unwrap_or(20.0))
    })
}

/// The invitation that is due, if any (once per period, counted from the
/// first frame in world).
pub fn tick() -> Option<NetEvent> {
    let every = every()?;
    static START: OnceLock<Instant> = OnceLock::new();
    static SENT: AtomicU32 = AtomicU32::new(0);
    let due = (START.get_or_init(Instant::now).elapsed().as_secs_f32() / every) as u32;
    let n = SENT.load(Ordering::Relaxed);
    if due <= n {
        return None;
    }
    SENT.store(n + 1, Ordering::Relaxed);
    let (from, name) = if n.is_multiple_of(2) {
        (DEMO_TESS, "Tess Touch".to_owned())
    } else {
        (DEMO_FRIEND, aurora_i18n::tr!("demo-orion-example").to_string())
    };
    log::info!("demo conference: invitation {} from {name}", n + 1);
    Some(NetEvent::SessionInvite(aurora_net::SessionInvite {
        session: Uuid::from_u128(SESSION | (n as u128 + 1)),
        from,
        from_name: name.clone(),
        message: aurora_i18n::tr!("demo-conference-invitation").to_string(),
        session_name: name,
        offline: false,
        timestamp: 0,
    }))
}
