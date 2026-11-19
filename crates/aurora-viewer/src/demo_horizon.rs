//! Planet curvature in the offline demo
//! (`AURORA_DEMO_HORIZON=<eye height m>[:flat|:<radius km>]`): the setting is
//! turned on with the Earth's radius (or the one given; `flat` keeps it off,
//! for before / after), and the camera stands over the sea at the demo
//! region's southern edge, `height` metres above the water, looking level to
//! the south over the void water. Flat, the sea runs out to the middle of the
//! picture; curved, it ends at the horizon's dip under it (0.1° from 10 m,
//! 0.56° from 300 m on the Earth), with the sky below.

use glam::Vec3;
use std::sync::OnceLock;

/// The demo region's water height (demo.rs).
const WATER: f32 = 20.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scenario {
    /// The camera's height above the water (m).
    pub height: f32,
    /// The planet's radius (km); None: flat.
    pub radius_km: Option<f32>,
}

/// The scenario asked for, None outside demo mode or when unset.
pub fn scenario() -> Option<Scenario> {
    static S: OnceLock<Option<Scenario>> = OnceLock::new();
    *S.get_or_init(|| {
        std::env::var_os("AURORA_DEMO")?;
        parse(&std::env::var("AURORA_DEMO_HORIZON").ok()?)
    })
}

fn parse(value: &str) -> Option<Scenario> {
    let mut parts = value.trim().split(':');
    let height = parts
        .next()?
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|h| h.is_finite())?
        .clamp(1.0, 3000.0);
    let radius_km = match parts.next().map(str::trim) {
        None | Some("") => Some(crate::settings::PLANET_RADIUS_KM),
        Some("flat") => None,
        Some(r) => Some(r.parse::<f32>().ok().filter(|r| r.is_finite() && *r > 0.0)?),
    };
    Some(Scenario { height, radius_km })
}

/// The setting as the scenario wants it (`apply_test_overrides`).
pub fn apply(settings: &mut crate::settings::Settings) {
    let Some(s) = scenario() else {
        return;
    };
    settings.planet_curvature = s.radius_km.is_some();
    if let Some(r) = s.radius_km {
        settings.planet_radius_km = r;
    }
}

/// Where the demo camera stands and looks.
pub fn view() -> Option<(Vec3, Vec3)> {
    let s = scenario()?;
    let eye = Vec3::new(128.0, 1.0, WATER + s.height);
    Some((eye, eye + Vec3::new(0.0, -1000.0, 0.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_and_planets_are_read() {
        let earth = Some(crate::settings::PLANET_RADIUS_KM);
        assert_eq!(
            parse("10"),
            Some(Scenario {
                height: 10.0,
                radius_km: earth
            })
        );
        assert_eq!(
            parse("300:flat"),
            Some(Scenario {
                height: 300.0,
                radius_km: None
            })
        );
        assert_eq!(
            parse(" 25 : 200 "),
            Some(Scenario {
                height: 25.0,
                radius_km: Some(200.0)
            })
        );
        assert_eq!(
            parse("0.1"),
            Some(Scenario {
                height: 1.0,
                radius_km: earth
            })
        );
        assert_eq!(parse("high"), None);
        assert_eq!(parse("10:-5"), None);
    }
}
