//! Night sky in the offline demo
//! (`AURORA_DEMO_NIGHT_SKY=real|legacy[:<azimuth>,<elevation>]`): the
//! demo's midnight (unless `AURORA_DEMO_TOD` says otherwise), the chosen
//! stars (Preferences › Graphics › Environment, "Night sky", in memory only)
//! and the camera looking at the sky, by default north 35° up: the Pole Star
//! with the Plough and Cassiopeia on either side. Azimuth in degrees from
//! north towards east, elevation in degrees above the horizon.

use crate::world::night_sky::NightSky;
use glam::Vec3;

/// The scenario: which stars, and where the camera looks (a unit
/// direction, x east, y north, z up). `None` when off or not understood.
pub fn scenario() -> Option<(NightSky, Vec3)> {
    parse(&std::env::var("AURORA_DEMO_NIGHT_SKY").ok()?)
}

fn parse(v: &str) -> Option<(NightSky, Vec3)> {
    let (mode, view) = v.trim().split_once(':').unwrap_or((v.trim(), ""));
    let mode = match mode {
        "real" | "1" => NightSky::Real,
        "legacy" => NightSky::Legacy,
        _ => return None,
    };
    let numbers: Vec<f32> = view.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let (azimuth, elevation) = match numbers[..] {
        [a, e] => (a, e.clamp(-10.0, 89.0)),
        _ => (0.0, 35.0),
    };
    let (a, e) = (azimuth.to_radians(), elevation.to_radians());
    Some((mode, Vec3::new(a.sin() * e.cos(), a.cos() * e.cos(), e.sin())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scenario_reads_the_stars_and_where_to_look() {
        let (mode, dir) = parse("real").expect("real");
        assert_eq!(mode, NightSky::Real);
        assert!(dir.y > 0.8 && dir.x.abs() < 1e-6 && dir.z > 0.5, "north, 35° up: {dir}");
        let (mode, dir) = parse("legacy:90,10").expect("legacy");
        assert_eq!(mode, NightSky::Legacy);
        assert!(dir.x > 0.98 && dir.z > 0.17, "east, 10° up: {dir}");
        assert_eq!(parse("stars"), None);
    }
}
