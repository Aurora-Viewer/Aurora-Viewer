//! People › Near you in Aurora's look (AURORA_DEMO_PEOPLE=rows|crowd|sheet|
//! legacy, docs/TESTING.md): the residents of the People mock-up around the
//! start position, at its ranges and in its states. `rows`: the rich rows
//! grouped by range; `sheet`: the same with the sheet of a friend open (the
//! window grows for it); `crowd`: forty residents in the compact density;
//! `legacy`: the same residents in Firestorm's radar table; `hover` and
//! `crowd-hover`: a friend with every badge under the pointer.
//!
//! The residents behind us are drawn (typing, sitting, away and do not
//! disturb come from their animations, as on the grid); the farther ones
//! are coarse locations only. Our friends of the scenario are the wolf of
//! the plaza ("Orion le Grand" here) and the first friend of the demo list.

use super::{DEMO_LOUP, HANDLE, IDLE_ANIM, floor_at, prim, shape, te};
use crate::world::radar::{ANIM_AGENT_SIT_GROUND, ANIM_AGENT_TYPE};
use crate::world::status::{ANIM_AWAY, ANIM_DO_NOT_DISTURB};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Rows,
    /// `rows`, the wolf under the pointer with every badge.
    Hover,
    Crowd,
    /// `crowd`, Aria under the pointer with every badge.
    CrowdHover,
    Sheet,
    Legacy,
}

pub fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_PEOPLE").ok()?.trim() {
        "hover" => Some(Mode::Hover),
        "crowd" | "foule" => Some(Mode::Crowd),
        "crowd-hover" => Some(Mode::CrowdHover),
        "sheet" | "fiche" => Some(Mode::Sheet),
        "legacy" => Some(Mode::Legacy),
        _ => Some(Mode::Rows),
    })
}

/// The crowd's density: `crowd` and `crowd-hover`.
pub fn compact() -> bool {
    matches!(mode(), Some(Mode::Crowd | Mode::CrowdHover))
}

/// Every badge a friend can carry: voice, typing, sitting, payment info,
/// note (and the friend's heart).
fn every_badge(state: &mut State) {
    state.voice = Some(true);
    state.typing = true;
    state.sitting = true;
    state.pays = true;
    state.note = true;
}

/// `legacy`: Firestorm's table for this run, whatever the preference says.
pub fn legacy() -> bool {
    mode() == Some(Mode::Legacy)
}

/// The first friend of the demo buddy list (a display name "Nova ♪").
pub const NOVA: Uuid = super::DEMO_FRIEND;

const fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xA0E0_9E09_0000_0000_0000_0000_0000_0000 | n)
}

#[derive(Clone, Copy, Default)]
struct State {
    away: bool,
    busy: bool,
    typing: bool,
    sitting: bool,
    /// In the voice channel: speaking (true) or silent.
    voice: Option<bool>,
    /// Payment info on file.
    pays: bool,
    /// We wrote a note on their profile.
    note: bool,
}

struct Resident {
    id: Uuid,
    first: &'static str,
    last: &'static str,
    /// Chosen display name; empty for none.
    display: &'static str,
    /// Meters from us on the ground; None for a coarse location without
    /// altitude.
    range: Option<f32>,
    /// Drawn (exact place, animations) or a coarse location only.
    drawn: bool,
    /// Minutes since the arrival.
    minutes: f64,
    state: State,
    /// Gradient of the profile picture.
    picture: Option<u8>,
}

/// The mock-up's list: three whispers, two more within the chat, the shouts,
/// then the far ones.
fn rich() -> Vec<Resident> {
    let example = aurora_i18n::tr!("demo-example");
    let s = |f: fn(&mut State)| {
        let mut st = State::default();
        f(&mut st);
        st
    };
    let r = |id, first, last, display, range: f32, minutes, state, picture| Resident {
        id,
        first,
        last,
        display,
        range: Some(range),
        drawn: true,
        minutes,
        state,
        picture,
    };
    let far = |n, first, range: Option<f32>, minutes| Resident {
        id: id(n),
        first,
        last: example,
        display: "",
        range,
        drawn: false,
        minutes,
        state: State::default(),
        picture: Some((n % 4) as u8),
    };
    vec![
        // the wolf of the plaza stands there already, 4.7 m away
        r(
            DEMO_LOUP,
            "Orion",
            example,
            aurora_i18n::tr!("demo-orion-the-great"),
            4.7,
            2.0,
            s(|s| {
                s.busy = true;
                s.voice = Some(true);
                s.pays = true;
            }),
            Some(1),
        ),
        r(
            id(1),
            "Lune",
            example,
            "",
            6.9,
            5.0,
            s(|s| {
                s.typing = true;
                s.pays = true;
            }),
            Some(2),
        ),
        r(
            id(2),
            "tesstouch",
            "Resident",
            "Tess ✧",
            8.1,
            8.0,
            s(|s| {
                s.voice = Some(true);
                s.note = true;
            }),
            Some(0),
        ),
        r(
            id(3),
            "Brume",
            example,
            "La Brume",
            8.0,
            11.0,
            s(|s| {
                s.away = true;
                s.pays = true;
            }),
            Some(0),
        ),
        r(NOVA, "Nova", example, "Nova ♪", 9.8, 14.0, s(|s| s.voice = Some(true)), Some(3)),
        r(
            id(5),
            "Maxi",
            "Ville",
            "Maximilienne ♪",
            17.6,
            17.0,
            s(|s| {
                s.sitting = true;
                s.pays = true;
            }),
            Some(2),
        ),
        r(id(6), "Iris", example, "", 24.6, 20.0, s(|s| s.typing = true), Some(2)),
        r(id(7), "Sol", example, "Sol ♪", 25.5, 23.0, s(|s| s.voice = Some(true)), Some(0)),
        r(
            id(8),
            "Atlas",
            example,
            "",
            42.0,
            26.0,
            s(|s| {
                s.away = true;
                s.sitting = true;
            }),
            Some(1),
        ),
        far(9, "Vega", Some(63.0), 31.0),
        far(10, "Kiri", Some(88.0), 35.0),
        far(11, "Oda", Some(96.0), 41.0),
        far(12, "Pax", Some(108.0), 47.0),
        far(13, "Rhea", Some(116.0), 52.0),
        far(14, "Zed", Some(119.0), 58.0),
        far(15, "Wren", None, 64.0),
    ]
}

/// The mock-up's crowd: forty names, ranges growing slowly then faster.
fn crowd() -> Vec<Resident> {
    const FIRST: [&str; 40] = [
        "Aria", "Bram", "Cléo", "Dax", "Elin", "Faro", "Gaïa", "Hugo", "Ines", "Jun", "Kiri", "Lior", "Mira", "Nils", "Oda", "Pax",
        "Quinn", "Rhea", "Sven", "Tali", "Ugo", "Vela", "Wren", "Xan", "Yuna", "Zed", "Ama", "Bix", "Coral", "Dune", "Eko", "Fen", "Gil",
        "Hale", "Isa", "Jax", "Koa", "Luz", "Moss", "Nyx",
    ];
    let last = [aurora_i18n::tr!("demo-example"), "Démo", "Lumen", "Brisé", "Nord"];
    FIRST
        .iter()
        .enumerate()
        .map(|(i, first)| {
            let k = i % 10;
            let state = State {
                away: i % 8 == 3,
                busy: i % 8 == 5,
                typing: k == 2 || k == 9,
                sitting: k == 6,
                voice: (k == 0 || k == 5 || k == 9).then_some(true),
                pays: k == 3 || k == 7,
                note: k == 7,
            };
            let i = i as f32;
            Resident {
                id: id(100 + i as u128),
                first,
                last: last[i as usize % last.len()],
                display: "",
                range: Some(2.0 + i * i * 0.055 + i * 1.3),
                drawn: true,
                minutes: f64::from(1.0 + i * 1.5),
                state,
                picture: Some((i as usize % 4) as u8),
            }
        })
        .collect()
}

/// The residents of the scenario (asked every frame by the voice levels).
fn residents() -> &'static [Resident] {
    static RESIDENTS: std::sync::OnceLock<Vec<Resident>> = std::sync::OnceLock::new();
    RESIDENTS.get_or_init(|| {
        let mut list = match mode() {
            None => Vec::new(),
            Some(_) if compact() => crowd(),
            Some(_) => rich(),
        };
        // the row under the pointer carries every badge
        if let Some(r) = hovered()
            .filter(|_| matches!(mode(), Some(Mode::Hover | Mode::CrowdHover)))
            .and_then(|h| list.iter_mut().find(|r| r.id == h))
        {
            every_badge(&mut r.state);
        }
        list
    })
}

/// Our friends in the scenario: (id, rights we grant, rights we have).
pub fn friends() -> Vec<(Uuid, i32, i32)> {
    match mode() {
        None => Vec::new(),
        Some(_) if compact() => [100, 110, 120].into_iter().map(|n| (id(n), 3, 1)).collect(),
        // Nova: online status and map granted, her map and objects received
        Some(_) => vec![(NOVA, 3, 7), (DEMO_LOUP, 7, 3)],
    }
}

/// The resident whose sheet opens with `sheet`.
pub fn sheet() -> Option<Uuid> {
    (mode() == Some(Mode::Sheet)).then_some(NOVA)
}

/// The row shown selected and under the pointer with `rows`, as in the
/// mock-up: Tess.
pub fn hovered() -> Option<Uuid> {
    match mode()? {
        Mode::Rows => Some(id(2)),
        Mode::Hover => Some(DEMO_LOUP),
        Mode::CrowdHover => Some(id(100)),
        _ => None,
    }
}

/// Names of the scenario's residents: (first, last, chosen display name).
pub fn name(agent: &Uuid) -> Option<(&'static str, &'static str, &'static str)> {
    let r = residents().iter().find(|r| r.id == *agent)?;
    Some((r.first, r.last, r.display))
}

/// Minutes since each resident arrived, for the radar's first sightings.
pub fn arrivals() -> Vec<(Uuid, f64)> {
    residents().iter().map(|r| (r.id, r.minutes * 60.0)).collect()
}

/// Their profile: payment info, a young account, a note, a picture.
pub fn profile(p: &mut aurora_net::AvatarProfile) {
    use aurora_net::profile::flags;
    let Some(r) = residents().iter().find(|r| r.id == p.id) else {
        return;
    };
    p.flags = if r.state.pays { flags::IDENTIFIED } else { 0 };
    p.notes = Some(if r.state.note {
        aurora_i18n::tr!("demo-met-at-the-beach-very-friendly").to_owned()
    } else {
        String::new()
    });
    if r.first == "tesstouch" {
        // three days old: the age alert's red in the sheet
        let today = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |d| d.as_secs_f64());
        p.born = Some(today - 3.5 * 86_400.0);
    }
    p.hide_age = Some(false);
    if let Some(gradient) = r.picture {
        p.sl_image = super::tags::picture_texture(gradient);
    }
}

/// Who is in the voice channel, and who speaks: (raw level, speaking).
pub fn voice(levels: &mut HashMap<Uuid, (f32, bool)>, time: f64) {
    for r in residents() {
        if let Some(speaking) = r.state.voice {
            // silent in the channel, or talking in short bursts
            let level = if speaking {
                0.4 + 0.2 * (time * 3.0).sin().abs() as f32
            } else {
                0.0
            };
            levels.insert(r.id, (level, speaking));
        }
    }
}

/// Where we stand (demo::events), and the way the residents are laid out:
/// behind us, so that their name plates stay out of the view.
const START: Vec2 = Vec2::new(132.0, 126.0);

/// The local id of the wolf of the plaza (demo::events: 9000 is ours).
const WOLF_LOCAL_ID: u32 = 9001;

/// Where an avatar's center stands above the ground.
const BODY: f32 = 0.84;

/// A place `range` meters from us (in space, the ground's slope counted)
/// inside the region, behind us: the k-th resident a little apart from the
/// previous one.
fn place(k: usize, range: f32) -> Vec3 {
    let me = START.extend(floor_at(START.x, START.y) + BODY);
    let mut angle = std::f32::consts::PI + 0.46 + ((k % 9) as f32 - 4.0) * 0.32;
    for _ in 0..24 {
        // the height between us takes its share of the range
        let mut ground = range;
        let mut at = START + Vec2::from_angle(angle) * ground;
        for _ in 0..3 {
            let dz = floor_at(at.x, at.y) + BODY - me.z;
            ground = (range * range - dz * dz).max(0.0).sqrt();
            at = START + Vec2::from_angle(angle) * ground;
        }
        if (4.0..252.0).contains(&at.x) && (4.0..252.0).contains(&at.y) {
            return at.extend(floor_at(at.x, at.y) + BODY);
        }
        angle += 0.3;
    }
    me
}

/// The residents: the drawn ones, then where everyone stands for the region
/// (its coarse locations, which also give their status dots). The wolf of
/// the plaza keeps his place, 4.7 m away.
pub fn events() -> Vec<NetEvent> {
    if mode().is_none() {
        return Vec::new();
    }
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects = Vec::new();
    let mut coarse = Vec::new();
    for (k, r) in residents().iter().enumerate() {
        let pos = place(k, r.range.unwrap_or(150.0));
        coarse.push((r.id, if r.range.is_some() { pos } else { pos.with_z(0.0) }));
        if !r.drawn || r.id == DEMO_LOUP {
            continue;
        }
        let mut a = prim(
            120_000 + k as u32,
            pos,
            Quat::from_rotation_z(k as f32 * 0.9),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([1.0; 4], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        a.pcode = LL_PCODE_LEGACY_AVATAR;
        a.full_id = r.id;
        a.name_values = format!("FirstName STRING RW SV {}\nLastName STRING RW SV {}", r.first, r.last);
        objects.push(a);
    }
    vec![
        NetEvent::ObjectUpdates { handle: HANDLE, objects },
        NetEvent::CoarseLocations {
            handle: HANDLE,
            positions: coarse,
        },
    ]
}

/// The states of the drawn residents (away, do not disturb, typing,
/// sitting: their animations) and our friends online, sent once the demo's
/// own start is over (it sets the wolf's animations and the friends).
pub fn state_events(time: f64, sent: &mut bool) -> Vec<NetEvent> {
    if *sent || mode().is_none() || time < 2.0 {
        return Vec::new();
    }
    *sent = true;
    let mut events: Vec<NetEvent> = residents()
        .iter()
        .filter(|r| r.drawn)
        .map(|r| {
            let mut anims = vec![(IDLE_ANIM, 1)];
            for (on, anim) in [
                (r.state.away, ANIM_AWAY),
                (r.state.busy, ANIM_DO_NOT_DISTURB),
                (r.state.typing, ANIM_AGENT_TYPE),
                (r.state.sitting, ANIM_AGENT_SIT_GROUND),
            ] {
                if on {
                    anims.push((anim, 1));
                }
            }
            NetEvent::AvatarAnimations {
                avatar: r.id,
                anims,
                sources: Vec::new(),
            }
        })
        .collect();
    events.push(NetEvent::FriendsOnline {
        ids: friends().into_iter().map(|(id, _, _)| id).collect(),
        online: true,
    });
    // the crowd is the mock-up's forty: the wolf of the plaza leaves
    if compact() {
        events.push(NetEvent::ObjectsKilled {
            handle: HANDLE,
            local_ids: vec![WOLF_LOCAL_ID],
        });
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crowd_spreads_inside_the_region() {
        let me = START.extend(floor_at(START.x, START.y) + BODY);
        for (k, r) in crowd().iter().enumerate() {
            let range = r.range.expect("a range");
            let at = place(k, range);
            assert!((at.distance(me) - range).abs() < 0.2, "{k}: {range} m, at {}", at.distance(me));
        }
        assert_eq!(crowd().len(), 40);
    }
}
