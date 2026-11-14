//! Multi-resident inventory share (#3074): seed many Orion-like friends
//! so the Share picker can exercise multi-select (Ctrl+click / Ctrl+A).
//!
//! `AURORA_DEMO_SHARE=multi` — 80 friends named like Orion Example, nearby
//! and in Search. `multi:n=200` (capped at 2_000) changes the count.
//! Opens the inventory with a giveable item selected; use Give / Share and
//! the Friends or Nearby tab (or search "ori").

use super::{HANDLE, IDLE_ANIM, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_net::land::FoundAvatar;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use uuid::Uuid;

/// Prefix of the synthetic agents (low 48 bits are 1..=n).
const PREFIX: u128 = 0xA0E0_5A4E_0000_0000_0000_0000_0000_0000;
const DEFAULT_COUNT: usize = 80;
const MAX_COUNT: usize = 2_000;

/// Start standing spot of the demo plaza (same as People / radar).
const START: Vec2 = Vec2::new(132.0, 126.0);
const BODY: f32 = 0.84;

/// Parse `AURORA_DEMO_SHARE` (`multi`, `multi:n=200`).
pub fn parse_count(raw: &str) -> Option<usize> {
    let mut parts = raw.split(':');
    let head = parts.next()?.trim();
    if !matches!(head, "multi" | "many" | "1") {
        return None;
    }
    let mut n = DEFAULT_COUNT;
    for part in parts {
        let part = part.trim();
        if let Some(raw) = part.strip_prefix("n=").or_else(|| part.strip_prefix("N="))
            && let Ok(parsed) = raw.parse::<usize>()
            && parsed > 0
        {
            n = parsed.min(MAX_COUNT);
        }
    }
    Some(n)
}

/// `AURORA_DEMO_SHARE=multi` or `multi:n=200`.
pub fn count() -> Option<usize> {
    static COUNT: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| std::env::var("AURORA_DEMO_SHARE").ok().as_deref().and_then(parse_count))
}

pub fn enabled() -> bool {
    count().is_some()
}

pub fn agent(i: usize) -> Uuid {
    Uuid::from_u128(PREFIX | (i as u128 + 1))
}

/// Index `0..n` when `id` is one of our synthetic Orion agents.
pub fn index_of(id: &Uuid) -> Option<usize> {
    let v = id.as_u128();
    if v & !0xFFFF_FFFF_FFFF != PREFIX {
        return None;
    }
    let n = (v & 0xFFFF_FFFF_FFFF) as usize;
    (n >= 1).then_some(n - 1)
}

/// Buddy-list rows: (id, rights given, rights has).
pub fn friends() -> Vec<(Uuid, i32, i32)> {
    let Some(n) = count() else {
        return Vec::new();
    };
    (0..n).map(|i| (agent(i), 3, 1)).collect()
}

/// Display-name parts: (legacy first, legacy last, chosen display).
pub fn name(id: &Uuid) -> Option<(String, String, String)> {
    let i = index_of(id)?;
    let n = count()?;
    if i >= n {
        return None;
    }
    let example = aurora_i18n::tr!("demo-example");
    let display = format!("Orion {example} {:03}", i + 1);
    Some(("Orion".into(), example.to_owned(), display))
}

/// Avatar search hits for the Share picker's Search tab.
pub fn avatar_search(query: &str) -> Option<Vec<FoundAvatar>> {
    let n = count()?;
    search_hits(n, query)
}

fn search_hits(n: usize, query: &str) -> Option<Vec<FoundAvatar>> {
    let q = query.trim().to_lowercase();
    if q.chars().count() < 3 {
        return Some(Vec::new());
    }
    let stem = q.starts_with("ori") || "orion".contains(&q) || q.contains("example") || q.contains("exemple");
    if !stem {
        return Some(Vec::new());
    }
    let example = aurora_i18n::tr!("demo-example");
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let id = agent(i);
        let display = format!("Orion {example} {:03}", i + 1);
        out.push(FoundAvatar {
            id,
            display_name: display,
            username: format!("orion.{}", example.to_lowercase()),
        });
    }
    Some(out)
}

/// Nearby Orion clones inside NearMeRange (162 m), for the picker's Nearby tab.
pub fn events() -> Vec<NetEvent> {
    let Some(n) = count() else {
        return Vec::new();
    };
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects = Vec::new();
    let mut coarse = Vec::new();
    for i in 0..n {
        let id = agent(i);
        // Ring ~8–40 m so every row is in NearMeRange and scrollable.
        let angle = (i as f32) * 0.37;
        let dist = 8.0 + (i % 17) as f32 * 1.8;
        let at = START + Vec2::from_angle(angle) * dist;
        let pos = Vec3::new(at.x.clamp(4.0, 252.0), at.y.clamp(4.0, 252.0), floor_at(at.x, at.y) + BODY);
        coarse.push((id, pos));
        let mut a = prim(
            130_000 + i as u32,
            pos,
            Quat::from_rotation_z(angle),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([1.0; 4], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        a.pcode = LL_PCODE_LEGACY_AVATAR;
        a.full_id = id;
        let example = aurora_i18n::tr!("demo-example");
        a.name_values = format!("FirstName STRING RW SV Orion\nLastName STRING RW SV {example}");
        objects.push(a);
    }
    let mut events = vec![
        NetEvent::ObjectUpdates { handle: HANDLE, objects },
        NetEvent::CoarseLocations {
            handle: HANDLE,
            positions: coarse,
        },
        NetEvent::FriendsOnline {
            ids: (0..n).map(agent).collect(),
            online: true,
        },
    ];
    for i in 0..n {
        events.push(NetEvent::AvatarAnimations {
            avatar: agent(i),
            anims: vec![(IDLE_ANIM, 1)],
            sources: Vec::new(),
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_parses_count_and_ids() {
        assert_eq!(parse_count("multi"), Some(80));
        assert_eq!(parse_count("multi:n=200"), Some(200));
        assert_eq!(parse_count("multi:n=99999"), Some(MAX_COUNT));
        assert_eq!(parse_count("other"), None);
        assert_eq!(agent(0).as_u128() & 0xFFFF_FFFF_FFFF, 1);
        assert_eq!(agent(79).as_u128() & !PREFIX, 80);
    }

    #[test]
    fn search_returns_every_orion_for_ori() {
        let hits = search_hits(40, "ori").expect("hits");
        assert_eq!(hits.len(), 40);
        assert!(hits[0].display_name.starts_with("Orion"));
        assert!(search_hits(10, "zz").expect("empty").is_empty());
    }
}
