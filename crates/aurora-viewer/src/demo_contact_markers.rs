//! Contact-set markers (AURORA_DEMO_CONTACT_MARKERS=region|parcel|fade|body,
//! docs/TESTING.md): three sets, two with a symbol and one with a plain
//! dot, Family in circles and the others in diamonds, and four residents
//! known only by their coarse location: one circling the region, one behind
//! the camera, one far to the side and one of unknown altitude. With
//! AURORA_DEMO_TAGS, plate residents join the sets.

use super::HANDLE;
use crate::world::contact_sets::{MarkerScope, MarkerShape, Visuals};
use aurora_net::NetEvent;
use glam::Vec3;
use uuid::Uuid;

const fn id(n: u128) -> Uuid {
    Uuid::from_u128(0xA0E0_7A66_0000_0000_0000_0000_0000_0000 | n)
}

const CIRCLING: Uuid = id(1);
const BEHIND: Uuid = id(2);
const SIDE: Uuid = id(3);
const UNKNOWN_ALTITUDE: Uuid = id(4);
/// Seconds of a lap of the region.
const LAP: f64 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Region,
    Parcel,
    Fade,
    Body,
}

fn mode() -> Option<Mode> {
    static MODE: std::sync::OnceLock<Option<Mode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_DEMO_CONTACT_MARKERS").ok()?.trim() {
        "parcel" => Some(Mode::Parcel),
        "fade" => Some(Mode::Fade),
        "body" => Some(Mode::Body),
        _ => Some(Mode::Region),
    })
}

pub fn active() -> bool {
    mode().is_some()
}

/// The scenario's marker position. Set in every mode: the demo saves its
/// preferences, so `body` would otherwise outlive its run.
pub fn apply_preferences(settings: &mut crate::settings::Settings) {
    use crate::settings::MarkerAnchor;
    if let Some(mode) = mode() {
        settings.contact_marker_anchor = if mode == Mode::Body {
            MarkerAnchor::Body
        } else {
            MarkerAnchor::AboveName
        };
    }
}

/// The sets of the scenario, for this run: nothing is saved.
pub fn seed(world: &mut crate::world::World) {
    let Some(mode) = mode() else { return };
    let scope = if mode == Mode::Parcel {
        MarkerScope::Parcel
    } else {
        MarkerScope::Region
    };
    let fade = (mode == Mode::Fade).then_some(12);
    let plates = crate::demo::tags::MARKER_RESIDENTS;
    let sets = &mut world.contact_sets;
    sets.keep_in_memory();
    for (name, color, members, icon, marker_shape) in [
        (
            aurora_i18n::tr!("demo-family"),
            [0.957, 0.447, 0.714, 1.0],
            vec![plates[0], plates[1], SIDE],
            Some(0),
            MarkerShape::Circle,
        ),
        (
            "Clubbing",
            [0.369, 0.918, 0.831, 1.0],
            vec![plates[2], CIRCLING],
            None,
            MarkerShape::Diamond,
        ),
        (
            aurora_i18n::tr!("demo-shop"),
            [0.988, 0.827, 0.302, 1.0],
            vec![BEHIND, UNKNOWN_ALTITUDE],
            Some(1),
            MarkerShape::Diamond,
        ),
    ] {
        sets.add_set(name);
        sets.set_color(name, color);
        sets.add_to_set(&members, name, |_| true);
        sets.set_visuals(
            name,
            Visuals {
                // Family's marker takes its plate border, the others the plain one
                border: icon == Some(0),
                // Shop's its plate background
                background: icon == Some(1),
                marker: scope,
                marker_shape,
                marker_fade: fade,
                icon,
                ..Default::default()
            },
        );
    }
    if mode == Mode::Parcel {
        let me = world.agent.position;
        world.parcel = Some(crate::demo::parcel_visibility::parcel_at(me.x, me.y));
    }
}

/// The coarse locations of one second `t`, around `me` in the region.
pub fn coarse(me: Vec3, t: f64) -> NetEvent {
    // the look direction of the demo's start (demo::events)
    let ahead = glam::Vec2::new(0.8, 0.4).normalize();
    let around = |distance: f32, side: f32| {
        let at = me.truncate() + ahead * distance + ahead.perp() * side;
        at.clamp(glam::Vec2::splat(2.0), glam::Vec2::splat(253.0)).extend(me.z)
    };
    let lap = std::f64::consts::TAU * (t % LAP) / LAP;
    let circling = Vec3::new(128.0 + 90.0 * lap.cos() as f32, 128.0 + 90.0 * lap.sin() as f32, me.z);
    NetEvent::CoarseLocations {
        handle: HANDLE,
        positions: vec![
            (CIRCLING, circling),
            (BEHIND, around(-70.0, 8.0)),
            (SIDE, around(25.0, -110.0)),
            // byte 255: above what the message can say
            (UNKNOWN_ALTITUDE, around(100.0, 40.0).truncate().extend(1020.0)),
        ],
    }
}
