//! Name plates in every state and at the three distance levels
//! (AURORA_DEMO_TAGS=voice|status|cycle|wall|height|render, docs/TESTING.md): five
//! residents in a row in front of the start position, two farther (compact
//! pills) and two beyond (the name alone). A white wall stands behind the
//! two on the left, to see the glass over a bright background too.
//!
//! `voice`: out of the channel, in it and silent, speaking (a friend drawn
//! as too complex), typing, blocked. `status`: away, do not disturb, the
//! friend out of the channel, typing while away, blocked. `cycle` goes from
//! one to the other every six seconds. `wall`: the states of `voice`, the
//! wall in front of the two on the left, and two boxes with the same
//! floating text, one behind the wall: what the scene hides has no label
//! (ui/label_cover.rs).

use super::{HANDLE, IDLE_ANIM, floor_at, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Voice,
    Status,
    Cycle,
    Wall,
    Height,
    Render,
}

/// Seconds a set of states lasts with `cycle`.
const CYCLE: f64 = 6.0;

/// Explicit offline overrides; the demo keeps its own preferences.
pub fn apply_name_tag_preferences(settings: &mut crate::settings::Settings) -> bool {
    let Ok(mode) = std::env::var("AURORA_DEMO_NAME_TAGS") else {
        return false;
    };
    settings.show_own_name_tag = mode != "self-off" && mode != "off";
    settings.show_friend_name_tags = mode != "friends-off" && mode != "off";
    settings.show_other_name_tags = mode != "others-off" && mode != "off";
    settings.temporary_name_tags = matches!(mode.as_str(), "temporary" | "classic-temporary");
    settings.name_tag_show_seconds = 10.0;
    settings.name_tag_distance = 64.0;
    settings.classic_name_tags = mode == "classic-temporary";
    true
}

pub fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_TAGS").ok()?.trim() {
        "status" => Some(Mode::Status),
        "cycle" => Some(Mode::Cycle),
        "wall" => Some(Mode::Wall),
        "height" => Some(Mode::Height),
        "render" => Some(Mode::Render),
        _ => Some(Mode::Voice),
    })
}

/// The states shown at `time`: false for the voice set, true for the
/// status set.
fn status_set(mode: Mode, time: f64) -> bool {
    match mode {
        Mode::Voice | Mode::Wall | Mode::Height | Mode::Render => false,
        Mode::Status => true,
        Mode::Cycle => (time / CYCLE) as u64 % 2 == 1,
    }
}

const fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xA0E0_7A65_0000_0000_0000_0000_0000_0000 | n)
}

const NOVA: Uuid = id(1);
const TESS: Uuid = id(2);
/// A friend of the demo buddy list.
const ORION: Uuid = Uuid::from_u128(0xD0D0_0000_0000_0000_0000_0000_0000_0000 | 102);
const LUNE: Uuid = id(4);
const BRUME: Uuid = id(5);
const SOL: Uuid = id(6);
const IRIS: Uuid = id(7);
const VEGA: Uuid = id(8);
const ATLAS: Uuid = id(9);

pub const HEIGHT_ANIM: Uuid = id(261);

/// A translation deformer plus visible head sway and pelvis bobbing:
/// the tag must clear the taller body without moving with either motion.
pub fn height_animation(rig: &crate::scene::anim::Rig) -> aurora_assets::Animation {
    use aurora_assets::anim::{JointMotion, PosKey, RotKey};
    let mut anim = super::idle_animation();
    anim.joints
        .retain(|j| !["mTorso", "mPelvis", "mHead"].contains(&j.joint_name.as_str()));
    let torso = rig.local_pos[rig.names["mTorso"]] + Vec3::Z * 0.8;
    anim.joints.push(JointMotion {
        joint_name: "mTorso".into(),
        priority: 4,
        rot_keys: Vec::new(),
        pos_keys: vec![PosKey {
            time: 0.0,
            position: torso,
        }],
    });
    anim.joints.push(JointMotion {
        joint_name: "mPelvis".into(),
        priority: 4,
        rot_keys: Vec::new(),
        pos_keys: [(0.0, 0.0), (2.0, 0.12), (4.0, 0.0)]
            .into_iter()
            .map(|(time, z)| PosKey {
                time,
                position: Vec3::Z * z,
            })
            .collect(),
    });
    anim.joints.push(JointMotion {
        joint_name: "mHead".into(),
        priority: 4,
        rot_keys: [(0.0, -0.5), (2.0, 0.5), (4.0, -0.5)]
            .into_iter()
            .map(|(time, angle)| RotKey {
                time,
                rotation: Quat::from_rotation_y(angle),
            })
            .collect(),
        pos_keys: Vec::new(),
    });
    anim
}

struct Resident {
    id: Uuid,
    /// Meters ahead of the start position and to its left.
    ahead: f32,
    left: f32,
    first: &'static str,
    /// Chosen display name; empty for none.
    display: &'static str,
    title: &'static str,
    /// Gradient of the profile picture; None for a resident without one.
    picture: Option<u8>,
}

fn residents() -> [Resident; 9] {
    let wolves = aurora_i18n::tr!("demo-northern-wolves");
    let r = |id, ahead, left, first, display, title, picture| Resident {
        id,
        ahead,
        left,
        first,
        display,
        title,
        picture,
    };
    [
        r(NOVA, 6.5, 7.3, "Nova", "Nova ♪", wolves, Some(3)),
        r(TESS, 6.5, 4.9, "Tess", "Tess ✨", "Aurora Builders", None),
        r(ORION, 6.5, 2.4, "Orion", aurora_i18n::tr!("demo-orion-the-great"), wolves, Some(1)),
        r(LUNE, 6.5, -2.3, "Lune", "", "Aurora Builders", Some(2)),
        r(BRUME, 6.5, -4.7, "Brume", "La Brume", "", Some(0)),
        // the far ones to the right of the plaza's torus, which would hide
        // them and their names
        r(SOL, 23.0, 10.5, "Sol", "Sol ♪", wolves, None),
        r(IRIS, 23.0, -8.5, "Iris", "", "", None),
        r(VEGA, 46.0, -15.0, "Vega", "", "", None),
        r(ATLAS, 46.0, 16.5, "Atlas", "", "", None),
    ]
}

/// Texture of a demo profile picture.
pub(crate) const fn picture_texture(gradient: u8) -> Uuid {
    Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0100 | gradient as u128)
}

/// Profile picture of a resident of the scenario (ours included); nil
/// outside it and for the residents without one.
pub fn profile_picture(agent: Uuid) -> Uuid {
    if mode().is_none() {
        return Uuid::nil();
    }
    if agent == super::DEMO_AGENT {
        return picture_texture(0);
    }
    residents()
        .iter()
        .find(|r| r.id == agent)
        .and_then(|r| r.picture)
        .map_or(Uuid::nil(), picture_texture)
}

/// Profile picture of the residents of the profile scenarios
/// (AURORA_DEMO_PROFILE): ours, the friend and the wolf, in the colors of
/// the profile mock-up's banners.
pub fn scenario_picture(agent: Uuid) -> Uuid {
    match agent {
        a if a == super::DEMO_AGENT => picture_texture(0),
        a if a == super::DEMO_FRIEND => picture_texture(3),
        a if a == super::DEMO_LOUP => picture_texture(1),
        _ => Uuid::nil(),
    }
}

/// Pixels of a demo profile picture: a diagonal gradient with the outline
/// of a bust, 64 x 64.
pub fn picture_pixels(texture: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    const GRADIENTS: [([f32; 3], [f32; 3]); 4] = [
        ([139.0, 92.0, 246.0], [45.0, 212.0, 191.0]),
        ([245.0, 158.0, 11.0], [239.0, 68.0, 68.0]),
        ([56.0, 189.0, 248.0], [99.0, 102.0, 241.0]),
        ([244.0, 114.0, 182.0], [167.0, 139.0, 250.0]),
    ];
    let n = texture.as_u128() ^ picture_texture(0).as_u128();
    let (from, to) = *GRADIENTS.get(usize::try_from(n).ok()?)?;
    const N: u32 = 64;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32 / (N - 1) as f32, y as f32 / (N - 1) as f32);
            let t = (fx + fy) * 0.5;
            // a head over shoulders, in the dark blue of the brand
            let head = (fx - 0.5).hypot(fy - 0.36) < 0.17;
            let shoulders = fy > 0.62 && ((fx - 0.5) / 0.34).powi(2) + ((fy - 0.92) / 0.32).powi(2) < 1.0;
            let shade = if head || shoulders { 0.72 } else { 0.0 };
            let mix = |a: f32, b: f32, dark: f32| ((a + (b - a) * t) * (1.0 - shade) + dark * shade) as u8;
            px.extend_from_slice(&[mix(from[0], to[0], 7.0), mix(from[1], to[1], 11.0), mix(from[2], to[2], 31.0), 255]);
        }
    }
    Some((px, N, N))
}

/// Names of the scenario's residents: (first, last, chosen display name).
pub fn name(agent: &Uuid) -> Option<(&'static str, &'static str, &'static str)> {
    mode()?;
    let r = residents().into_iter().find(|r| r.id == *agent)?;
    Some((r.first, aurora_i18n::tr!("demo-example"), r.display))
}

/// Line of the block list for the blocked resident of the scenario.
pub fn mute_line() -> Option<String> {
    mode().map(|_| format!("1 {BRUME} Brume|0\n"))
}

/// The resident whose plate is shown as too complex.
pub fn too_complex() -> Option<Uuid> {
    mode().map(|_| ORION)
}

/// Exercise the production rendering exceptions on full and compact plates.
pub fn apply_render_preferences(settings: &mut crate::settings::Settings) {
    if mode() == Some(Mode::Render) {
        for id in [TESS, IRIS] {
            settings.set_render_exception(id, 1);
        }
    }
}

/// Where the scenario stands: the demo's start position (AURORA_DEMO_POS).
fn start() -> (f32, f32) {
    let pos: Vec<f32> = std::env::var("AURORA_DEMO_POS")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_default();
    match pos[..] {
        [x, y, ..] => (x, y),
        _ => (132.0, 126.0),
    }
}

/// The residents, standing in rows across the view of the start position.
pub fn events() -> Vec<NetEvent> {
    let (sx, sy) = start();
    // the look direction of the demo's start (demo::events)
    let ahead = glam::Vec2::new(0.8, 0.4).normalize();
    let left = glam::Vec2::new(-ahead.y, ahead.x);
    let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
    let mut objects = Vec::new();
    let mut events = Vec::new();
    for (k, r) in residents().iter().enumerate() {
        let at = glam::Vec2::new(sx, sy) + ahead * r.ahead + left * r.left;
        let mut a = prim(
            110_000 + k as u32,
            Vec3::new(at.x, at.y, floor_at(at.x, at.y) + 0.84),
            Quat::from_rotation_z(std::f32::consts::PI + 0.46),
            Vec3::new(0.45, 0.6, 1.9),
            boxp,
            te([1.0; 4], 0, false, 0.0),
            ExtraParams::default(),
            "",
        );
        a.pcode = LL_PCODE_LEGACY_AVATAR;
        a.full_id = r.id;
        let last = aurora_i18n::tr!("demo-example");
        a.name_values = format!("FirstName STRING RW SV {}\nLastName STRING RW SV {last}", r.first);
        if !r.title.is_empty() {
            a.name_values.push_str(&format!("\nTitle STRING RW SV {}", r.title));
        }
        objects.push(a);
        events.push(NetEvent::AvatarAnimations {
            avatar: r.id,
            anims: vec![(
                if mode() == Some(Mode::Height) && [NOVA, LUNE].contains(&r.id) {
                    HEIGHT_ANIM
                } else {
                    IDLE_ANIM
                },
                1,
            )],
            sources: Vec::new(),
        });
    }
    // a white wall behind the two on the left: the glass over a bright
    // background, next to the same plates over the darker scene; with
    // `wall` it stands in front of them and hides them, names included
    let hiding = mode() == Some(Mode::Wall);
    if hiding {
        // the same floating text behind the wall and in the open
        for (k, side) in [3.8, -3.0].into_iter().enumerate() {
            let at = glam::Vec2::new(sx, sy) + ahead * 4.4 + left * side;
            objects.push(prim(
                110_110 + k as u32,
                Vec3::new(at.x, at.y, floor_at(at.x, at.y) + 0.3),
                Quat::IDENTITY,
                Vec3::splat(0.6),
                boxp,
                te([0.55, 0.36, 0.96, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                aurora_i18n::tr!("demo-welcome-card"),
            ));
        }
    }
    let wall = glam::Vec2::new(sx, sy) + ahead * if hiding { 3.5 } else { 9.5 } + left * 6.1;
    objects.push(prim(
        110_100,
        Vec3::new(wall.x, wall.y, floor_at(wall.x, wall.y) + 1.9),
        Quat::from_rotation_z(ahead.y.atan2(ahead.x)),
        Vec3::new(0.1, 6.4, 3.8),
        boxp,
        te([1.0; 4], 0, true, 0.0),
        ExtraParams::default(),
        "",
    ));
    events.insert(0, NetEvent::ObjectUpdates { handle: HANDLE, objects });
    events
}

/// The animations that carry the states (away, do not disturb, typing),
/// sent when the set of states changes; `shown` remembers the last one.
/// Iris types without the typing animation (a viewer with PlayTypingAnim
/// off): only her typing notices say so.
pub fn state_events(time: f64, shown: &mut Option<bool>) -> Vec<NetEvent> {
    use crate::world::radar::ANIM_AGENT_TYPE;
    use crate::world::status::{ANIM_AWAY, ANIM_DO_NOT_DISTURB};
    let Some(mode) = mode() else { return Vec::new() };
    if mode == Mode::Height {
        return Vec::new();
    }
    let status = status_set(mode, time);
    if *shown == Some(status) {
        return Vec::new();
    }
    *shown = Some(status);
    let states: [(Uuid, &[Uuid]); 5] = if status {
        [
            (NOVA, &[ANIM_AWAY]),
            (TESS, &[ANIM_DO_NOT_DISTURB]),
            (LUNE, &[ANIM_AWAY, ANIM_AGENT_TYPE]),
            (IRIS, &[]),
            (SOL, &[]),
        ]
    } else {
        [(NOVA, &[]), (TESS, &[]), (LUNE, &[ANIM_AGENT_TYPE]), (IRIS, &[]), (SOL, &[])]
    };
    states
        .into_iter()
        .map(|(avatar, playing)| NetEvent::AvatarAnimations {
            avatar,
            anims: std::iter::once(IDLE_ANIM).chain(playing.iter().copied()).map(|a| (a, 1)).collect(),
            sources: Vec::new(),
        })
        .chain(std::iter::once(NetEvent::NearbyTyping {
            avatar: IRIS,
            typing: !status,
        }))
        .collect()
}

/// AURORA_DEMO_CHAT_BUBBLES=1|typing|talk (with AURORA_DEMO_TAGS): the
/// residents talk; the chat bubbles on for the run, the typing ones too with
/// `typing`; `talk` leaves both to Preferences.
fn chat_bubbles_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| std::env::var("AURORA_DEMO_CHAT_BUBBLES").ok().map(|v| v.trim().to_owned()))
        .as_deref()
}

/// The bubbles forced on for the run: Some(typing dots too).
pub fn chat_bubbles() -> Option<bool> {
    chat_bubbles_mode().filter(|m| *m != "talk").map(|m| m == "typing")
}

/// Seconds between two rounds of the scenario's chat, which starts
/// `CHAT_START` seconds in: every line has gone (24 s) before the next.
const CHAT_ROUND: f64 = 30.0;
const CHAT_START: f64 = 2.0;

/// AURORA_DEMO_CHAT_BUBBLES: what the residents say, every `CHAT_ROUND`
/// seconds: a line, a whisper, a shout, an emote, a long line that wraps,
/// ours, a far resident's, and the blocked one's (no bubble). Nova speaks
/// again later, her first lines greyed by then. Sends what is due since
/// `last`.
pub fn chat_events(time: f64, last: &mut f64) -> Vec<NetEvent> {
    use aurora_net::{ChatMessage, ChatSourceType, ChatType};
    if chat_bubbles_mode().is_none() || mode().is_none() {
        return Vec::new();
    }
    use aurora_i18n::tr;
    let script: [(f64, Uuid, ChatType, &str); 9] = [
        (0.0, NOVA, ChatType::Normal, tr!("demo-bubbles-local-1")),
        (0.6, ORION, ChatType::Whisper, tr!("demo-chat-bubbles-whisper")),
        (1.2, TESS, ChatType::Shout, tr!("demo-chat-lines-shout")),
        (1.8, LUNE, ChatType::Normal, tr!("demo-chat-lines-emote")),
        (2.4, super::DEMO_AGENT, ChatType::Normal, tr!("demo-bubbles-free")),
        (3.0, SOL, ChatType::Normal, tr!("demo-chat-lines-long")),
        (3.6, BRUME, ChatType::Normal, tr!("demo-bubbles-local-3")),
        (4.2, NOVA, ChatType::Normal, tr!("demo-bubbles-local-2")),
        (12.0, NOVA, ChatType::Normal, tr!("demo-bubbles-local-3")),
    ];
    let mut events = Vec::new();
    let first = ((*last - CHAT_START) / CHAT_ROUND).floor().max(0.0) as u64;
    let rounds = ((time - CHAT_START) / CHAT_ROUND).floor();
    if rounds >= 0.0 {
        for round in first..=rounds as u64 {
            for (offset, who, chat_type, text) in &script {
                let at = CHAT_START + round as f64 * CHAT_ROUND + offset;
                if at <= *last || at > time {
                    continue;
                }
                let name = if *who == super::DEMO_AGENT {
                    "Aurora Demo".to_owned()
                } else {
                    residents()
                        .iter()
                        .find(|r| r.id == *who)
                        .map(|r| format!("{} {}", r.first, aurora_i18n::tr!("demo-example")))
                        .unwrap_or_default()
                };
                events.push(NetEvent::Chat(ChatMessage {
                    from_name: name,
                    source_id: *who,
                    owner_id: *who,
                    source_type: ChatSourceType::Agent,
                    chat_type: *chat_type,
                    position: Vec3::ZERO,
                    message: (*text).to_owned(),
                }));
            }
        }
    }
    *last = time;
    events
}

/// Who is in the voice channel at `time`, and how loud: (raw level,
/// speaking), as `demo::voice_levels`.
pub fn voice(levels: &mut HashMap<Uuid, (f32, bool)>, time: f64) {
    let Some(mode) = mode() else { return };
    let t = time as f32;
    // syllables and short pauses, like the mockup's level
    let talk = |phase: f32| {
        let syllable = (t * 2.3 + phase).sin().abs() * (0.6 + 0.4 * (t * 7.1 + phase).sin());
        // a normal voice: green, short of a shout's red
        let level = 0.33 + 0.18 * syllable;
        let on = (t * 0.45 + phase).sin() > -0.55;
        if on { (level, true) } else { (0.0, false) }
    };
    levels.insert(SOL, talk(1.3));
    levels.insert(VEGA, (0.0, false));
    if !status_set(mode, time) {
        // AURORA_DEMO_VOICE=sweep: Tess speaks too, over the white wall,
        // from a whisper to a shout and back
        let tess = if sweep() { (voice_sweep(time), true) } else { (0.0, false) };
        levels.insert(TESS, tess);
        levels.insert(ORION, talk(0.0));
    }
}

/// AURORA_DEMO_VOICE=sweep: Tess's voice goes over the whole range of the
/// speaking ring (`voice_sweep`); our own plate follows the microphone.
fn sweep() -> bool {
    static SWEEP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SWEEP.get_or_init(|| std::env::var("AURORA_DEMO_VOICE").is_ok_and(|v| v.trim() == "sweep"))
}

/// Raw levels of a whisper (just above the speaking level), a normal voice
/// and a shout (past the overdriven level), on WebRTC's participant scale.
const WHISPER: f32 = 0.33;
const NORMAL: f32 = 0.46;
const SHOUT: f32 = 0.61;
/// Seconds of each step of `voice_sweep`: held, then eased to the next.
const SWEEP_STEP: f64 = 2.0;
const SWEEP_HOLD: f64 = 1.2;

/// A voice that goes whisper → normal → shout → normal and again, every
/// eight seconds, with a light syllable wobble: the whole range of the
/// speaking ring without a microphone.
fn voice_sweep(time: f64) -> f32 {
    let steps = [WHISPER, NORMAL, SHOUT, NORMAL];
    let at = time.max(0.0) / SWEEP_STEP;
    let i = at as usize % steps.len();
    let into = (at.fract() * SWEEP_STEP - SWEEP_HOLD).max(0.0) / (SWEEP_STEP - SWEEP_HOLD);
    let ease = (into * into * (3.0 - 2.0 * into)) as f32;
    let (from, to) = (steps[i], steps[(i + 1) % steps.len()]);
    // nine wobbles a loop
    let wobble = (time * std::f64::consts::TAU * 9.0 / 8.0).sin() as f32;
    from + (to - from) * ease + 0.01 * wobble
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_glow_sweep_covers_whisper_to_shout_without_a_jump() {
        use crate::ui::nameplate::overdrive;
        let levels: Vec<f32> = (0..=1600).map(|i| voice_sweep(i as f64 / 100.0)).collect();
        // always speaking, from a whisper to past the overdriven level
        assert!(levels.iter().all(|&l| l > aurora_voice::SPEAKING_AUDIO_LEVEL));
        assert!(levels.iter().any(|&l| l < WHISPER + 0.02) && levels.iter().any(|&l| overdrive(l) > 0.9));
        assert!(levels.iter().filter(|&&l| overdrive(l) > 0.0).count() < levels.len() / 2);
        for w in levels.windows(2) {
            assert!((w[1] - w[0]).abs() < 0.01, "{w:?}");
        }
        // eight seconds a loop
        assert!((voice_sweep(1.0) - voice_sweep(9.0)).abs() < 1e-3);
    }

    #[test]
    fn the_cycle_alternates_the_two_sets() {
        assert!(!status_set(Mode::Cycle, 1.0));
        assert!(status_set(Mode::Cycle, CYCLE + 1.0));
        assert!(!status_set(Mode::Cycle, 2.0 * CYCLE + 1.0));
        assert!(status_set(Mode::Status, 0.0) && !status_set(Mode::Voice, 100.0));
    }

    #[test]
    fn profile_pictures_are_opaque_squares() {
        for gradient in 0..4 {
            let (px, w, h) = picture_pixels(&picture_texture(gradient)).expect("a picture");
            assert_eq!(px.len(), (w * h * 4) as usize);
            assert!(px.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
        }
        assert!(picture_pixels(&picture_texture(4)).is_none());
        assert!(picture_pixels(&super::super::TEX_GRADIENT).is_none());
    }

    #[test]
    fn the_residents_stand_at_the_three_levels() {
        use crate::ui::nameplate::{COMPACT_DISTANCE, FULL_DISTANCE, LEVEL_MARGIN};
        // the camera is a few meters behind the start position
        let camera = 4.0;
        let rows: Vec<f32> = residents().iter().map(|r| r.ahead.hypot(r.left) + camera).collect();
        assert!(rows[..5].iter().all(|d| *d < FULL_DISTANCE), "{rows:?}");
        assert!(
            rows[5..7]
                .iter()
                .all(|d| *d > FULL_DISTANCE + LEVEL_MARGIN && *d < COMPACT_DISTANCE - LEVEL_MARGIN)
        );
        assert!(rows[7..].iter().all(|d| *d > COMPACT_DISTANCE + LEVEL_MARGIN));
    }
}
